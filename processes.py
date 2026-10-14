"""Child process trees that can always be stopped: renders and agent turns."""

from __future__ import annotations

import ctypes
from ctypes import wintypes
import logging
import os
import signal
import subprocess
import threading
import time


class _JobLimits(ctypes.Structure):
    _fields_ = [
        ("process_time", ctypes.c_int64),
        ("job_time", ctypes.c_int64),
        ("flags", wintypes.DWORD),
        ("minimum_working_set", ctypes.c_size_t),
        ("maximum_working_set", ctypes.c_size_t),
        ("active_processes", wintypes.DWORD),
        ("affinity", ctypes.c_size_t),
        ("priority", wintypes.DWORD),
        ("scheduling", wintypes.DWORD),
    ]


class _JobIO(ctypes.Structure):
    _fields_ = [
        (name, ctypes.c_uint64)
        for name in (
            "read_operations",
            "write_operations",
            "other_operations",
            "read_bytes",
            "write_bytes",
            "other_bytes",
        )
    ]


class _ExtendedJobLimits(ctypes.Structure):
    _fields_ = [
        ("basic", _JobLimits),
        ("io", _JobIO),
        ("process_memory", ctypes.c_size_t),
        ("job_memory", ctypes.c_size_t),
        ("peak_process_memory", ctypes.c_size_t),
        ("peak_job_memory", ctypes.c_size_t),
    ]


class _JobAccounting(ctypes.Structure):
    _fields_ = [
        ("user_time", ctypes.c_int64),
        ("kernel_time", ctypes.c_int64),
        ("period_user_time", ctypes.c_int64),
        ("period_kernel_time", ctypes.c_int64),
        ("page_faults", wintypes.DWORD),
        ("total_processes", wintypes.DWORD),
        ("active_processes", wintypes.DWORD),
        ("terminated_processes", wintypes.DWORD),
    ]


class WindowsProcessJob:
    """Own descendants even after their parent process exits."""

    def __init__(self, process):
        self.lock = threading.Lock()
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        for name, arguments, result in (
            ("CreateJobObjectW", [wintypes.LPVOID, wintypes.LPCWSTR], wintypes.HANDLE),
            (
                "SetInformationJobObject",
                [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD],
                wintypes.BOOL,
            ),
            (
                "OpenProcess",
                [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD],
                wintypes.HANDLE,
            ),
            (
                "AssignProcessToJobObject",
                [wintypes.HANDLE, wintypes.HANDLE],
                wintypes.BOOL,
            ),
            ("TerminateJobObject", [wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
            (
                "QueryInformationJobObject",
                [
                    wintypes.HANDLE,
                    ctypes.c_int,
                    wintypes.LPVOID,
                    wintypes.DWORD,
                    ctypes.POINTER(wintypes.DWORD),
                ],
                wintypes.BOOL,
            ),
            ("CloseHandle", [wintypes.HANDLE], wintypes.BOOL),
        ):
            function = getattr(self.kernel, name)
            function.argtypes, function.restype = arguments, result
        # Null security attributes make this handle noninheritable.
        self.handle = self.kernel.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            limits = _ExtendedJobLimits()
            limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            if not self.kernel.SetInformationJobObject(
                self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)
            ):
                raise ctypes.WinError(ctypes.get_last_error())
            process_handle = self.kernel.OpenProcess(0x0101, False, process.pid)
            if not process_handle:
                raise ctypes.WinError(ctypes.get_last_error())
            try:
                if not self.kernel.AssignProcessToJobObject(
                    self.handle, process_handle
                ):
                    raise ctypes.WinError(ctypes.get_last_error())
            finally:
                self.kernel.CloseHandle(process_handle)
        except Exception:
            self.close()
            raise

    def terminate(self):
        with self.lock:
            if self.handle and not self.kernel.TerminateJobObject(self.handle, 1):
                raise ctypes.WinError(ctypes.get_last_error())
            deadline = time.monotonic() + 5
            while self._active_count():
                if time.monotonic() >= deadline:
                    raise subprocess.TimeoutExpired("process tree", 5)
                time.sleep(0.01)

    def _active_count(self):
        if not self.handle:
            return 0
        accounting = _JobAccounting()
        if not self.kernel.QueryInformationJobObject(
            self.handle, 1, ctypes.byref(accounting), ctypes.sizeof(accounting), None
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        return accounting.active_processes

    def active(self):
        with self.lock:
            return bool(self._active_count())

    def close(self):
        with self.lock:
            if self.handle:
                if not self.kernel.CloseHandle(self.handle):
                    raise ctypes.WinError(ctypes.get_last_error())
                self.handle = None


def popen_options() -> dict:
    """Hide console windows on Windows; give POSIX children their own group."""
    if os.name == "nt":
        return {"creationflags": subprocess.CREATE_NO_WINDOW}
    return {"start_new_session": True}


def attach_process_tree(process):
    if os.name == "nt":
        process._studio_job = WindowsProcessJob(process)


def release_process_tree(process):
    job = process.__dict__.get("_studio_job")
    if job is not None:
        job.close()


def tree_active(process) -> bool:
    job = process.__dict__.get("_studio_job")
    return process.poll() is None or (job is not None and job.active())


def stop_process(process: subprocess.Popen | None):
    if process is None:
        return
    job = process.__dict__.get("_studio_job")
    if job is not None:
        job.terminate()
        return
    if os.name == "nt":
        if process.poll() is not None:
            return
        subprocess.run(
            ["taskkill", "/PID", str(process.pid), "/T", "/F"],
            capture_output=True,
            creationflags=subprocess.CREATE_NO_WINDOW,
            timeout=5,
        )
    else:
        try:
            # A child may hold stdout open after the process-group leader exits.
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def start_process_monitor(process):
    """Stop helpers a finished process leaves behind as soon as it exits."""

    def finish_children():
        try:
            process.wait()
            stop_process(process)
        except (OSError, subprocess.SubprocessError):
            logging.exception("Unable to stop remaining helper processes")
            try:
                release_process_tree(process)
            except OSError:
                logging.exception("Unable to close the process job")

    monitor = threading.Thread(target=finish_children, daemon=True)
    monitor.start()
    return monitor
