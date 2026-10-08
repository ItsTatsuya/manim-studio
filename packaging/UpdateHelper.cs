// Runs from the update cache, after Studio and its launcher release their files.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.IO.Compression;
using Microsoft.Win32.SafeHandles;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Runtime.Serialization;
using System.Runtime.Serialization.Json;
using System.Web.Script.Serialization;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Windows.Forms;
using System.Xml;

class UpdateHelper {
    [StructLayout(LayoutKind.Sequential)] struct FileTime { public uint Low, High; }
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetProcessTimes(SafeWaitHandle handle, out FileTime created, out FileTime exited, out FileTime kernel, out FileTime user);
    sealed class TrackedProcess : WaitHandle {
        public TrackedProcess(IntPtr handle) { SafeWaitHandle = new SafeWaitHandle(handle, true); }
        public bool HasExited { get { return WaitOne(0); } }
    }
    [DataContract] sealed class ReleaseIdentity {
        [DataMember(Name="version", IsRequired=true)] public string Version;
        [DataMember(Name="webview2")] public string WebView;
        [DataMember(Name="math")] public string Math;
    }
    sealed class DesktopLease : IDisposable {
        readonly Mutex singleton = new Mutex(false, "Local\\ManimStudio-Desktop");
        readonly Mutex updating = new Mutex(false, "Local\\ManimStudio-Update");
        bool desktopOwned, updateOwned;
        public DesktopLease() {
            try {
                try { updateOwned = updating.WaitOne(0); } catch (AbandonedMutexException) { updateOwned = true; }
                if (!updateOwned) throw new IOException("Another Studio update is in progress.");
                try { desktopOwned = singleton.WaitOne(TimeSpan.FromSeconds(30)); } catch (AbandonedMutexException) { desktopOwned = true; }
                if (!desktopOwned) throw new IOException("Another Studio window is open. Close it before updating.");
            } catch { Dispose(); throw; }
        }
        public void Dispose() {
            if (desktopOwned) { singleton.ReleaseMutex(); desktopOwned = false; }
            if (updateOwned) { updating.ReleaseMutex(); updateOwned = false; }
            singleton.Dispose(); updating.Dispose();
        }
    }
    static readonly HashSet<string> Components = new HashSet<string>(StringComparer.OrdinalIgnoreCase) {
        "app", "runtime", "math", "tools", "webview2", "licenses", "docs",
        "Manim Studio.exe", "standalone.json", "portable.mode", "BUILD-PROVENANCE.json",
        "README.md", "READ ME.txt", "LICENSE", "THIRD-PARTY-NOTICES.txt",
        "CONTRIBUTING.md", "SECURITY.md", "THIRD-PARTY-NOTICES.md"
    };
    static readonly string[] Required = {
        "app/desktop.py", "app/studio.py", "app/runtime.py", "app/worker.py",
        "app/updates.py", "app/update_handoff.py", "app/manim.cfg", "app/Studio Update.exe",
        "app/studio_ui/index.html", "app/studio_ui/app.js", "app/studio_ui/workspace.js",
        "app/studio_ui/updates.js", "app/studio_ui/style.css", "app/studio_ui/vendor/editor.bundle.js",
        "runtime/python.exe", "runtime/pythonw.exe", "runtime/python312.dll", "runtime/python312.zip",
        "runtime/python312._pth", "tools/ffmpeg.exe", "tools/ffprobe.exe",
        "Manim Studio.exe", "standalone.json", "portable.mode", "LICENSE", "THIRD-PARTY-NOTICES.txt"
    };

    static bool Below(string path, string parent) {
        return Path.GetFullPath(path).StartsWith(Path.GetFullPath(parent).TrimEnd('\\') + "\\", StringComparison.OrdinalIgnoreCase);
    }
    static string IoPath(string path) {
        // Keep journals and containment checks in ordinary canonical paths.
        // Only filesystem calls need the extended namespace, even when the
        // user's LongPathsEnabled policy is off (the Windows default).
        string full = Path.GetFullPath(path);
        if (full.StartsWith(@"\\?\", StringComparison.Ordinal)) return full;
        return full.StartsWith(@"\\", StringComparison.Ordinal)
            ? @"\\?\UNC\" + full.Substring(2) : @"\\?\" + full;
    }
    static void CheckTree(string path) {
        path = IoPath(path);
        if ((File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0)
            throw new IOException("Update files must not contain symbolic links or junctions: " + path);
        if (Directory.Exists(path))
            foreach (string item in Directory.EnumerateFileSystemEntries(path)) CheckTree(item);
    }
    static void Remove(string path) {
        path = IoPath(path);
        if (Directory.Exists(path)) Directory.Delete(path, true);
        else if (File.Exists(path)) File.Delete(path);
    }
    static void Move(string source, string target) {
        source = IoPath(source); target = IoPath(target);
        if (Directory.Exists(source)) Directory.Move(source, target);
        else File.Move(source, target);
    }
    static string Hash(Stream source) {
        using (SHA256 sha = SHA256.Create()) {
            string value = BitConverter.ToString(sha.ComputeHash(source)).Replace("-", "").ToLowerInvariant();
            source.Position = 0;
            return value;
        }
    }
    static void Wait(TrackedProcess process, DateTime deadline) {
        if (process == null) return;
        while (!process.WaitOne(250))
            if (DateTime.UtcNow >= deadline) throw new IOException("Studio is still open. Close it, then try the update again.");
    }
    static TrackedProcess OriginalProcess(int pid, long expectedTicks) {
        if (pid == 0) return null;
        // Capture an OS handle before reading creation time; the process may exit
        // during handoff, and the handle prevents PID reuse from changing identity.
        IntPtr handle = OpenProcess(0x00101000, false, pid);
        if (handle == IntPtr.Zero) {
            int error = Marshal.GetLastWin32Error();
            if (error == 87) return null;
            throw new Win32Exception(error);
        }
        TrackedProcess process = new TrackedProcess(handle);
        try {
            FileTime created, exited, kernel, user;
            if (!GetProcessTimes(process.SafeWaitHandle, out created, out exited, out kernel, out user)) throw new Win32Exception(Marshal.GetLastWin32Error());
            long started = checked((long)(((ulong)created.High << 32) | created.Low) + 504911232000000000L);
            if (expectedTicks != 0 && started != expectedTicks) { process.Dispose(); return null; }
            return process;
        } catch { process.Dispose(); throw; }
    }
    static void Launch(string root) {
        ProcessStartInfo start = new ProcessStartInfo(Path.Combine(root, "Manim Studio.exe"));
        start.WorkingDirectory = root;
        start.UseShellExecute = false;
        if (!String.IsNullOrEmpty(Environment.GetEnvironmentVariable("MANIM_STUDIO_DATA"))) {
            start.EnvironmentVariables["MANIM_STUDIO_UPDATE_ROOT"] = root;
            start.EnvironmentVariables["MANIM_STUDIO_UPDATE_DATA"] = Environment.GetEnvironmentVariable("MANIM_STUDIO_DATA");
        }
        using (Process process = Process.Start(start)) {
            if (process.WaitForExit(1500) && process.ExitCode != 0)
                throw new IOException("The updated app could not start.");
        }
    }
    static void Extract(Stream stream, string stage, string version) {
        var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        long total = 0;
        using (ZipArchive zip = new ZipArchive(stream, ZipArchiveMode.Read, true)) {
            if (zip.Entries.Count > 100000) throw new IOException("The update archive has too many files.");
            foreach (ZipArchiveEntry entry in zip.Entries) {
                const string prefix = "Manim Studio/";
                if (!entry.FullName.StartsWith(prefix, StringComparison.Ordinal) || entry.FullName.Contains("\\"))
                    throw new IOException("Unexpected update archive layout.");
                string relative = entry.FullName.Substring(prefix.Length).TrimEnd('/');
                if (relative.Length == 0) continue;
                string[] parts = relative.Split('/');
                foreach (string part in parts)
                    if (part.Length == 0 || part == "." || part == ".." || part.EndsWith(".") || part.EndsWith(" ") ||
                        part.IndexOfAny(Path.GetInvalidFileNameChars()) >= 0 ||
                        Regex.IsMatch(part, @"^(CON|PRN|AUX|NUL|COM[1-9\u00b9\u00b2\u00b3]|LPT[1-9\u00b9\u00b2\u00b3])(\.|$)", RegexOptions.IgnoreCase))
                        throw new IOException("Unsafe update archive filename.");
                if (!Components.Contains(parts[0]) || !names.Add(relative) ||
                    ((entry.ExternalAttributes >> 16) & 0xf000) == 0xa000)
                    throw new IOException("The update contains unsupported files or duplicate paths.");
                total = checked(total + entry.Length);
                if (relative == "standalone.json" && entry.Length > 65536) throw new IOException("The release metadata is unexpectedly large.");
                if (total > 8L * 1024 * 1024 * 1024 || entry.Length > 2L * 1024 * 1024 * 1024)
                    throw new IOException("The extracted update is unexpectedly large.");
                string path = Path.GetFullPath(Path.Combine(stage, relative.Replace('/', '\\')));
                if (!Below(path, stage)) throw new IOException("Unsafe update archive path.");
                if (entry.FullName.EndsWith("/")) Directory.CreateDirectory(IoPath(path));
                else {
                    Directory.CreateDirectory(IoPath(Path.GetDirectoryName(path)));
                    using (Stream input = entry.Open())
                    using (FileStream output = new FileStream(IoPath(path), FileMode.CreateNew, FileAccess.Write, FileShare.None)) input.CopyTo(output);
                }
            }
        }
        foreach (string name in Required)
            if (!File.Exists(IoPath(Path.Combine(stage, name.Replace('/', '\\')))) || new FileInfo(IoPath(Path.Combine(stage, name.Replace('/', '\\')))).Length == 0) throw new IOException("The update is incomplete: " + name);
        using (FileStream metadata = File.OpenRead(IoPath(Path.Combine(stage, "standalone.json")))) {
            // Framework's data-contract reader accepts a truncated closing brace.
            // Validate the full document first, then enforce the root identity contract.
            new JavaScriptSerializer { MaxJsonLength = 65536, RecursionLimit = 64 }.DeserializeObject(File.ReadAllText(IoPath(Path.Combine(stage, "standalone.json"))));
            var serializer = new DataContractJsonSerializer(typeof(ReleaseIdentity));
            using (XmlDictionaryReader reader = JsonReaderWriterFactory.CreateJsonReader(metadata, XmlDictionaryReaderQuotas.Max)) {
                ReleaseIdentity identity = (ReleaseIdentity)serializer.ReadObject(reader);
                while (reader.Read()) { }
                if (identity.Version != version) throw new IOException("The archive version does not match the selected release.");
                string[] browserFiles;
                if (identity.WebView == null || identity.WebView == "fixed") browserFiles = new[] {"webview2/msedgewebview2.exe"};
                else if (identity.WebView == "evergreen") browserFiles = new[] {"app/webview_runtime.py", "app/webview-evergreen.json", "tools/MicrosoftEdgeWebview2Setup.exe"};
                else throw new IOException("Unsupported browser runtime profile.");
                foreach (string file in browserFiles) RequireFile(stage, file);
                if (identity.Math == "none") RequireFile(stage, "runtime/Lib/site-packages/typst/_typst.pyd");
                else if (identity.Math == null || identity.Math == "minimal" || identity.Math == "full") {
                    RequireFile(stage, "math/texmfs/install/miktex/bin/x64/latex.exe");
                    RequireFile(stage, "math/texmfs/install/miktex/bin/x64/dvisvgm.exe");
                } else throw new IOException("Unsupported equation runtime profile.");
            }
        }
    }
    static void RequireFile(string stage, string name) {
        string path = IoPath(Path.Combine(stage, name.Replace('/', '\\')));
        if (!File.Exists(path) || new FileInfo(path).Length == 0) throw new IOException("The update is incomplete: " + name);
    }
    static bool Exists(string path) { path = IoPath(path); return File.Exists(path) || Directory.Exists(path); }
    static string Journal(string root) { return Path.Combine(root, "update.pending"); }
    static void Phase(FileStream lease, string phase) {
        byte[] value = Encoding.UTF8.GetBytes(phase + "\n");
        lease.Seek(0, SeekOrigin.End); lease.Write(value, 0, value.Length); lease.Flush(true);
    }
    static void Restore(string root, string backup, List<string> plan) {
        for (int i = plan.Count - 1; i >= 0; i--) {
            string[] item = plan[i].Split('|');
            string target = Path.Combine(root, item[0]), previous = Path.Combine(backup, item[0]);
            if (Exists(previous)) {
                CheckTree(previous);
                if (File.Exists(IoPath(previous)) && File.Exists(IoPath(target))) File.Replace(IoPath(previous), IoPath(target), null);
                else { if (Exists(target)) { CheckTree(target); Remove(target); } Move(previous, target); }
            } else if (item[1] == "0" && Exists(target)) { CheckTree(target); Remove(target); }
            else if (item[1] == "1" && !Exists(target)) throw new IOException("A previous app component is missing: " + target);
        }
    }
    static void Recover(string root) {
        string marker = Journal(root);
        if (!File.Exists(marker)) return;
        string stage;
        using (FileStream lease = new FileStream(marker, FileMode.Open, FileAccess.ReadWrite, FileShare.None)) {
            string raw;
            using (StreamReader reader = new StreamReader(lease, Encoding.UTF8, true, 1024, true)) raw = reader.ReadToEnd();
            string[] lines = raw.Replace("\r\n", "\n").Split(new[] {'\n'}, StringSplitOptions.RemoveEmptyEntries);
            if (lines.Length < 5 || lines[0] != "MANIM-STUDIO-UPDATE-1" || !String.Equals(Path.GetFullPath(lines[1]), root, StringComparison.OrdinalIgnoreCase))
                throw new IOException("The update recovery journal is invalid. Restore the previous app backup manually.");
            stage = Path.GetFullPath(lines[2]);
            if (!String.Equals(Path.GetDirectoryName(stage), Path.GetDirectoryName(root), StringComparison.OrdinalIgnoreCase) ||
                !Regex.IsMatch(Path.GetFileName(stage), @"^\.manim-update-[a-f0-9]{32}$")) throw new IOException("Unsafe update recovery directory.");
            if (Directory.Exists(IoPath(stage))) CheckTree(stage);
            var plan = new List<string>();
            var recorded = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            for (int i = 4; i < lines.Length; i++) {
                if (lines[i] == "commit" || lines[i] == "rollback") continue;
                // A power interruption may tear the final flushed phase append.
                // Conservatively roll back a partial phase; never accept a partial component plan.
                if (i == lines.Length - 1 && ("commit".StartsWith(lines[i], StringComparison.Ordinal) || "rollback".StartsWith(lines[i], StringComparison.Ordinal))) {
                    int ending = raw.EndsWith("\r\n", StringComparison.Ordinal) ? 2 : raw.EndsWith("\n", StringComparison.Ordinal) ? 1 : 0;
                    lease.SetLength(lease.Length - Encoding.UTF8.GetByteCount(lines[i]) - ending);
                    lease.Flush(true);
                    continue;
                }
                string[] record = lines[i].Split('|');
                if (record.Length != 2 || !Components.Contains(record[0]) || !recorded.Add(record[0]) || (record[1] != "0" && record[1] != "1")) throw new IOException("Invalid update component journal.");
                plan.Add(lines[i]);
            }
            if (lines[lines.Length - 1] != "commit") {
                Phase(lease, "rollback");
                Restore(root, Path.Combine(stage, "previous"), plan);
            }
            if (Directory.Exists(IoPath(stage))) Directory.Delete(IoPath(stage), true);
        }
        File.Delete(marker);
    }
    sealed class PortableUpdate : IDisposable {
        readonly string root, stage, incoming, backup;
        readonly List<string> plan = new List<string>();
        bool journaled;
        public PortableUpdate(Stream source, string target, string version) {
            root = target;
            if (!File.Exists(Path.Combine(root, "portable.mode"))) throw new IOException("This folder is not a portable edition.");
            if (File.Exists(Journal(root))) {
                string[] prior = File.ReadAllLines(Journal(root));
                if (prior.Length == 0 || prior[prior.Length - 1] != "commit") throw new IOException("An interrupted update needs recovery. Reopen Studio using Manim Studio.exe before updating.");
                // A committed transaction can only delete its old staging files.
                Recover(root);
            }
            ReclaimPreflight(root);
            string cacheName = Path.GetFileName(AppDomain.CurrentDomain.BaseDirectory.TrimEnd('\\'));
            if (!Regex.IsMatch(cacheName, "^install-[a-f0-9]{32}$")) throw new IOException("Invalid update cache identity.");
            stage = Path.Combine(Path.GetDirectoryName(root), ".manim-update-" + cacheName.Substring(8));
            if (Directory.Exists(IoPath(stage))) throw new IOException("This update staging directory is already in use.");
            using (Process owner = Process.GetCurrentProcess()) File.WriteAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "stage.owner"), root + "\n" + stage + "\n" + owner.Id + "\n" + owner.StartTime.ToUniversalTime().Ticks + "\n");
            incoming = Path.Combine(stage, "incoming"); backup = Path.Combine(stage, "previous");
            Directory.CreateDirectory(IoPath(incoming));
            try {
                Extract(source, incoming, version);
                var paths = new List<string>(Directory.GetFileSystemEntries(IoPath(incoming)));
                // The launcher remains present throughout the transaction so it can recover a power interruption.
                paths.Sort((a,b) => {
                    bool al = Path.GetFileName(a) == "Manim Studio.exe", bl = Path.GetFileName(b) == "Manim Studio.exe";
                    return al == bl ? StringComparer.Ordinal.Compare(a,b) : al ? 1 : -1;
                });
                foreach (string path in paths) {
                    string name = Path.GetFileName(path), current = Path.Combine(root, name);
                    if (Exists(current)) CheckTree(current);
                    plan.Add(name + "|" + (Exists(current) ? "1" : "0"));
                }
            } catch { Directory.Delete(IoPath(stage), true); throw; }
        }
        public void Apply() {
            Directory.CreateDirectory(IoPath(backup));
            string temporary = Path.Combine(stage, "journal.tmp");
            string content = "MANIM-STUDIO-UPDATE-1\n" + root + "\n" + stage + "\n" + Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "Studio Update.exe") + "\n" + String.Join("\n", plan) + "\n";
            using (FileStream file = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None)) {
                byte[] value = Encoding.UTF8.GetBytes(content); file.Write(value,0,value.Length); file.Flush(true);
            }
            File.Move(temporary, Journal(root)); journaled = true;
            using (FileStream lease = new FileStream(Journal(root), FileMode.Open, FileAccess.ReadWrite, FileShare.Read)) {
                try {
                    foreach (string record in plan) {
                        string name = record.Split('|')[0], next = Path.Combine(incoming, name), target = Path.Combine(root, name), old = Path.Combine(backup, name);
                        if (File.Exists(IoPath(next)) && File.Exists(IoPath(target))) File.Replace(IoPath(next), IoPath(target), IoPath(old));
                        else {
                            if (Exists(target)) Move(target, old);
                            Move(next, target);
                        }
                    }
                    Phase(lease, "commit");
                } catch {
                    Phase(lease, "rollback");
                    try { Restore(root, backup, plan); }
                    catch (Exception rollback) { throw new IOException("Your previous app files remain in " + backup + ". Reopen Studio to recover this update. UserData was preserved.", rollback); }
                    throw;
                }
            }
            try { Recover(root); journaled = false; }
            catch (IOException error) { File.AppendAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"update.log"), "Update succeeded; cleanup will retry next time: " + error.Message + "\n"); }
        }
        public void Dispose() {
            if (!journaled && Directory.Exists(IoPath(stage))) { CheckTree(stage); Directory.Delete(IoPath(stage), true); }
        }
    }
    static void ReclaimPreflight(string root) {
        string cache = Path.Combine(root, "UserData", ".studio", "updates");
        if (!Directory.Exists(cache)) return;
        foreach (string directory in Directory.EnumerateDirectories(cache, "install-*")) {
            string name = Path.GetFileName(directory), marker = Path.Combine(directory, "stage.owner");
            if (!Regex.IsMatch(name, "^install-[a-f0-9]{32}$") || !File.Exists(marker)) continue;
            if ((File.GetAttributes(directory) & FileAttributes.ReparsePoint) != 0 || (File.GetAttributes(marker) & FileAttributes.ReparsePoint) != 0) continue;
            string[] record = File.ReadAllLines(marker);
            string expected = Path.Combine(Path.GetDirectoryName(root), ".manim-update-" + name.Substring(8));
            if (record.Length != 4 || !String.Equals(record[0], root, StringComparison.OrdinalIgnoreCase) || !String.Equals(record[1], expected, StringComparison.OrdinalIgnoreCase)) continue;
            int pid; long ticks;
            if (!int.TryParse(record[2], out pid) || !long.TryParse(record[3], out ticks)) continue;
            using (TrackedProcess owner = OriginalProcess(pid, ticks)) if (owner != null && !owner.HasExited) continue;
            if (Directory.Exists(IoPath(expected))) { CheckTree(expected); Directory.Delete(IoPath(expected), true); }
            File.Delete(marker);
        }
    }
    [STAThread] static int Main(string[] args) {
        // Staging adds a suffix to paths that already fit in the installed tree.
        // Opt into .NET's long-path APIs before any Path/File operations; the
        // helper runs on Windows 10's .NET 4.8 and needs no copied config file.
        AppContext.SetSwitch("Switch.System.IO.UseLegacyPathHandling", false);
        AppContext.SetSwitch("Switch.System.IO.BlockLongPaths", false);
        string log = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "update.log");
        bool preparedForClose = false;
        try {
            if ((args.Length == 2 || args.Length == 4 || (args.Length == 3 && args[2] == "--no-relaunch")) && args[0] == "recover") {
                string recoveryRoot = Path.GetFullPath(args[1]).TrimEnd('\\');
                if (args.Length == 4)
                    using (TrackedProcess launcher = OriginalProcess(int.Parse(args[2], CultureInfo.InvariantCulture), long.Parse(args[3], CultureInfo.InvariantCulture))) Wait(launcher, DateTime.UtcNow.AddSeconds(60));
                using (DesktopLease admission = new DesktopLease()) Recover(recoveryRoot);
                if (args.Length == 4) Launch(recoveryRoot);
                return 0;
            }
            bool test = args.Length == 10 && args[9] == "--no-relaunch";
            if (args.Length != 9 && !test) throw new ArgumentException("Invalid update request.");
            string mode = args[0], root = Path.GetFullPath(args[1]).TrimEnd('\\'), artifact = Path.GetFullPath(args[2]);
            if (!File.Exists(Path.Combine(root, "standalone.json")) || !File.Exists(Path.Combine(root, "app", "desktop.py")))
                throw new IOException("The Studio installation is incomplete.");
            if ((File.GetAttributes(root) & FileAttributes.ReparsePoint) != 0) throw new IOException("Update the resolved app folder instead of a junction.");
            if (!Regex.IsMatch(args[3], "^[a-f0-9]{64}$") || !Regex.IsMatch(args[4], @"^[0-9]+\.[0-9]+\.[0-9]+$"))
                throw new IOException("Invalid verified release identity.");
            using (TrackedProcess studio = OriginalProcess(int.Parse(args[5], CultureInfo.InvariantCulture), long.Parse(args[6], CultureInfo.InvariantCulture)))
            using (TrackedProcess launcher = OriginalProcess(int.Parse(args[7], CultureInfo.InvariantCulture), long.Parse(args[8], CultureInfo.InvariantCulture)))
            using (FileStream source = new FileStream(artifact, FileMode.Open, FileAccess.Read, FileShare.Read)) {
                if (Hash(source) != args[3]) throw new IOException("The download changed after verification. Download it again.");
                if (mode == "portable") {
                    using (PortableUpdate prepared = new PortableUpdate(source, root, args[4])) {
                        File.WriteAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "update.ready"), "ready\n");
                        preparedForClose = true;
                        DateTime deadline = DateTime.UtcNow.AddSeconds(60);
                        Wait(studio, deadline); Wait(launcher, deadline);
                        using (DesktopLease admission = new DesktopLease()) prepared.Apply();
                        if (!test) Launch(root);
                    }
                }
                else if (mode == "installed") {
                    if (File.Exists(Path.Combine(root, "portable.mode"))) throw new IOException("Portable editions require the ZIP update.");
                    File.WriteAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "update.ready"), "ready\n");
                    DateTime deadline = DateTime.UtcNow.AddSeconds(60);
                    Wait(studio, deadline); Wait(launcher, deadline);
                    using (DesktopLease admission = new DesktopLease()) {
                        ProcessStartInfo setup = new ProcessStartInfo(artifact, "/SILENT /NORESTART /NOCLOSEAPPLICATIONS /NORESTARTAPPLICATIONS /DIR=\"" + root + "\"");
                        setup.UseShellExecute = false;
                        using (Process process = Process.Start(setup)) {
                            process.WaitForExit();
                            if (process.ExitCode != 0) throw new IOException("Setup did not finish (code " + process.ExitCode + "). Your saved work is preserved.");
                        }
                    }
                    if (!test) Launch(root);
                } else throw new IOException("Unknown update edition.");
            }
            File.WriteAllText(log, "Update completed: " + args[4] + Environment.NewLine);
            return 0;
        } catch (Exception error) {
            try { File.WriteAllText(log, error.ToString()); } catch (IOException) { }
            if (preparedForClose && args.Length == 9 && args[0] == "portable") {
                try { string root = Path.GetFullPath(args[1]).TrimEnd('\\'); if (File.Exists(Journal(root))) { using (DesktopLease admission = new DesktopLease()) Recover(root); Launch(root); } }
                catch (Exception recovery) { try { File.AppendAllText(log, "\nRecovery: " + recovery); } catch (IOException) { } }
            }
            if (args.Length == 0 || args[args.Length - 1] != "--no-relaunch")
                MessageBox.Show(error.Message + "\n\nDetails: " + log, "Manim Studio update could not finish", MessageBoxButtons.OK, MessageBoxIcon.Error);
            return 1;
        }
    }
}
