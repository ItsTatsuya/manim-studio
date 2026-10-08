using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

[assembly: AssemblyTitle("Manim Studio")]
[assembly: AssemblyDescription("Local animation studio")]
[assembly: AssemblyProduct("Manim Studio")]
[assembly: AssemblyVersion("1.0.0.0")]
[assembly: AssemblyFileVersion("1.0.0.0")]

class Launcher {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr window, int mode);

    static string Quote(string value) {
        StringBuilder quoted = new StringBuilder("\""); int slashes = 0;
        foreach (char c in value) {
            if (c == '\\') { slashes++; continue; }
            if (c == '"') { quoted.Append('\\', slashes * 2 + 1); quoted.Append(c); }
            else { quoted.Append('\\', slashes); quoted.Append(c); }
            slashes = 0;
        }
        quoted.Append('\\', slashes * 2); quoted.Append('"'); return quoted.ToString();
    }

    [STAThread] static int Main(string[] args) {
        Application.EnableVisualStyles();
        using (Mutex updating = new Mutex(false, "Local\\ManimStudio-Update")) {
            bool available;
            try { available = updating.WaitOne(0); } catch (AbandonedMutexException) { available = true; }
            if (!available) return 0;
            updating.ReleaseMutex();
        }
        bool ownsMutex;
        using (Mutex singleton = new Mutex(true, "Local\\ManimStudio-Desktop", out ownsMutex)) {
            if (!ownsMutex && args.Length == 0) {
                IntPtr window = FindWindow(null, "Manim Studio");
                if (window != IntPtr.Zero) { ShowWindow(window, 9); SetForegroundWindow(window); }
                return 0;
            }
            try {
                string root = AppDomain.CurrentDomain.BaseDirectory;
                string pending = Path.Combine(root, "update.pending");
                if (File.Exists(pending)) {
                    string[] journal;
                    using (FileStream file = new FileStream(pending, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
                    using (StreamReader reader = new StreamReader(file)) {
                        journal = reader.ReadToEnd().Replace("\r\n", "\n").Split(new[] {'\n'}, StringSplitOptions.RemoveEmptyEntries);
                    }
                    if (journal.Length < 5 || journal[0] != "MANIM-STUDIO-UPDATE-1") throw new IOException("The update recovery journal is incomplete. Restore the previous app backup before opening Studio.");
                    if (journal[journal.Length - 1] != "commit") {
                        string helper = Path.GetFullPath(journal[3]);
                        string cache = Path.GetFullPath(Path.Combine(root, "UserData", ".studio", "updates")) + Path.DirectorySeparatorChar;
                        if (!helper.StartsWith(cache, StringComparison.OrdinalIgnoreCase) || Path.GetFileName(helper) != "Studio Update.exe" || !File.Exists(helper)) throw new IOException("The interrupted update helper is missing. Restore the previous app backup.");
                        using (Process launcher = Process.GetCurrentProcess()) {
                            ProcessStartInfo recovery = new ProcessStartInfo(helper, "recover " + Quote(root) + " " + launcher.Id + " " + launcher.StartTime.ToUniversalTime().Ticks);
                            recovery.UseShellExecute = false; recovery.CreateNoWindow = true;
                            using (Process restore = Process.Start(recovery)) { }
                        }
                        // Recovery may replace this executable; release its image and mutex first.
                        return 0;
                    }
                }
                bool portable = File.Exists(Path.Combine(root, "portable.mode"));
                string data = portable ? Path.Combine(root, "UserData") : Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "ManimStudio", "Data");
                string updateRoot = Environment.GetEnvironmentVariable("MANIM_STUDIO_UPDATE_ROOT");
                string updateData = Environment.GetEnvironmentVariable("MANIM_STUDIO_UPDATE_DATA");
                if (!String.IsNullOrEmpty(updateRoot) && !String.IsNullOrEmpty(updateData) && Path.IsPathRooted(updateData) &&
                    String.Equals(Path.GetFullPath(updateRoot).TrimEnd('\\'), Path.GetFullPath(root).TrimEnd('\\'), StringComparison.OrdinalIgnoreCase)) data = Path.GetFullPath(updateData);
                Directory.CreateDirectory(data);
                string browserRoot = Path.Combine(root, "webview2");
                string aclMarker = Path.Combine(data, "browser-acl-ready");
                string browserIdentity = Path.GetFullPath(browserRoot).TrimEnd(Path.DirectorySeparatorChar).ToUpperInvariant();
                if (Environment.OSVersion.Version.Build < 22000 && (!File.Exists(aclMarker) || File.ReadAllText(aclMarker) != browserIdentity)) {
                    foreach (string sid in new string[] { "*S-1-15-2-1:(OI)(CI)(RX)", "*S-1-15-2-2:(OI)(CI)(RX)" }) {
                        ProcessStartInfo acl = new ProcessStartInfo("icacls.exe", Quote(browserRoot) + " /grant " + Quote(sid) + " /Q");
                        acl.UseShellExecute = false; acl.CreateNoWindow = true;
                        using(Process setup = Process.Start(acl)) { setup.WaitForExit(); if(setup.ExitCode != 0) throw new IOException("Studio could not prepare its browser folder. Extract the app to a folder you own and try again."); }
                    }
                    File.WriteAllText(aclMarker, browserIdentity);
                }
                string python = Path.Combine(root, "runtime", "pythonw.exe");
                string script = Path.Combine(root, "app", "desktop.py");
                if (!File.Exists(python) || !File.Exists(script)) throw new IOException("Extract the entire portable ZIP, then open Manim Studio.exe inside the extracted folder.");
                StringBuilder command = new StringBuilder(Quote(script));
                foreach(string argument in args) command.Append(" " + Quote(argument));
                ProcessStartInfo start = new ProcessStartInfo(python, command.ToString());
                start.UseShellExecute = false; start.CreateNoWindow = true; start.WorkingDirectory = root;
                start.EnvironmentVariables["MANIM_STUDIO_DATA"] = data;
                start.EnvironmentVariables.Remove("MANIM_STUDIO_UPDATE_ROOT");
                start.EnvironmentVariables.Remove("MANIM_STUDIO_UPDATE_DATA");
                using (Process launcher = Process.GetCurrentProcess()) {
                    start.EnvironmentVariables["MANIM_STUDIO_LAUNCHER_PID"] = launcher.Id.ToString();
                    start.EnvironmentVariables["MANIM_STUDIO_LAUNCHER_STARTED"] = launcher.StartTime.ToUniversalTime().Ticks.ToString();
                }
                start.EnvironmentVariables["PYTHONNOUSERSITE"] = "1";
                start.EnvironmentVariables["PYTHONUTF8"] = "1";
                start.EnvironmentVariables.Remove("PYTHONHOME");
                start.EnvironmentVariables.Remove("PYTHONPATH");
                start.EnvironmentVariables.Remove("VIRTUAL_ENV");
                using (Process process = Process.Start(start)) { process.WaitForExit(); return process.ExitCode; }
            } catch(Exception error) {
                Console.Error.WriteLine(error.ToString());
                MessageBox.Show(error.Message, "Manim Studio needs a hand", MessageBoxButtons.OK, MessageBoxIcon.Error);
                return 1;
            } finally { if (ownsMutex) singleton.ReleaseMutex(); }
        }
    }
}
