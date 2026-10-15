import { Outlet, useRouterState } from "@tanstack/react-router";
import { MenuIcon, XIcon } from "lucide-react";
import { lazy, Suspense, useEffect, useState } from "react";

import { applyTheme, normalizeTheme, watchSystemTheme } from "~/lib/theme";
import { cn } from "~/lib/utils";
import { connectEvents, useStudio } from "~/state/studio";
import { AppSidebar } from "./sidebar/AppSidebar";
import { ConfirmDialogHost } from "./ui/dialog";
import { Button } from "./ui/button";
import { Spinner } from "./ui/input";
import { Toaster } from "./ui/toast";

// First-run setup is only needed once, so it is not part of the main bundle.
const Onboarding = lazy(() =>
  import("./Onboarding").then((module) => ({ default: module.Onboarding })),
);
import { TooltipProvider } from "./ui/tooltip";
import { StudioMark } from "./icons";

export function AppShell() {
  // Warm the editor and markdown chunks once the first screen is painted, so
  // opening a script or chat never waits on the network or disk.
  useEffect(() => {
    const timer = window.setTimeout(() => {
      void import("./panel/CodeEditor");
      void import("./chat/Markdown");
    }, 200);
    return () => window.clearTimeout(timer);
  }, []);
  const ready = useStudio((state) => state.ready);
  const theme = useStudio((state) =>
    normalizeTheme(state.prefs["manim-theme"]),
  );
  const [failed, setFailed] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  // The rail only exists on desktop; the mobile drawer is always full width, so
  // it never reshapes itself while it slides away.
  const [desktop, setDesktop] = useState(
    () => window.matchMedia("(min-width: 768px)").matches,
  );
  // Hidden from focus and assistive tech only after the drawer finished sliding
  // out, so closing still animates and resizing never leaves it half-visible.
  const [parked, setParked] = useState(true);
  useEffect(() => {
    if (sidebarOpen) {
      setParked(false);
      return;
    }
    const reduced = window.matchMedia(
      "(prefers-reduced-motion: reduce)",
    ).matches;
    const timer = window.setTimeout(() => setParked(true), reduced ? 0 : 230);
    return () => window.clearTimeout(timer);
  }, [sidebarOpen]);
  // Width animates only for a click on the toggle, never on load or resize.
  const [animateSidebar, setAnimateSidebar] = useState(false);
  const sidebarCollapsed = useStudio(
    (state) => state.prefs["studio-sidebar-collapsed"] === "true",
  );
  useEffect(() => {
    if (!animateSidebar) return;
    const timer = window.setTimeout(() => setAnimateSidebar(false), 300);
    return () => window.clearTimeout(timer);
  }, [animateSidebar, sidebarCollapsed]);
  const installing = useStudio((state) => state.installing);
  const completedSetup = useStudio(
    (state) => state.prefs["studio-onboarding"] === "2",
  );
  const [reopenSetup, setReopenSetup] = useState(false);
  useEffect(() => {
    const reopen = () => setReopenSetup(true);
    window.addEventListener("studio:setup", reopen);
    return () => window.removeEventListener("studio:setup", reopen);
  }, []);
  const location = useRouterState({ select: (state) => state.location.href });
  useEffect(() => setSidebarOpen(false), [location]);
  useEffect(() => {
    if (!sidebarOpen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setSidebarOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sidebarOpen]);
  useEffect(() => {
    const query = window.matchMedia("(min-width: 768px)");
    const onChange = () => {
      setDesktop(query.matches);
      if (query.matches) setSidebarOpen(false);
    };
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let disconnect: (() => void) | undefined;
    useStudio
      .getState()
      .bootstrap()
      .then(() => {
        if (!cancelled) disconnect = connectEvents();
      })
      .catch((error: Error) => {
        if (!cancelled) setFailed(error.message);
      });
    return () => {
      cancelled = true;
      disconnect?.();
    };
  }, []);

  useEffect(() => {
    if (ready) applyTheme(theme);
  }, [ready, theme]);
  useEffect(
    () =>
      watchSystemTheme(() =>
        normalizeTheme(useStudio.getState().prefs["manim-theme"]),
      ),
    [],
  );

  if (failed) {
    return (
      <div className="grid h-full place-items-center p-8">
        <div className="max-w-sm text-center">
          <StudioMark className="mx-auto mb-4 size-10" />
          <h1 className="font-semibold text-lg">Studio couldn't start</h1>
          <p className="mt-2 text-muted-foreground text-sm">{failed}</p>
          <Button className="mt-5" onClick={() => window.location.reload()}>
            Try again
          </Button>
        </div>
      </div>
    );
  }
  if (!ready) {
    return (
      <div className="grid h-full place-items-center">
        <div className="flex items-center gap-2.5 text-muted-foreground text-sm">
          <Spinner /> Opening Manim Studio…
        </div>
      </div>
    );
  }
  return (
    <TooltipProvider delay={400}>
      <div className="flex h-full min-h-0" inert={installing}>
        <button
          aria-label="Close projects"
          tabIndex={sidebarOpen ? 0 : -1}
          className={cn(
            "fixed inset-0 z-30 bg-black/50 transition-[opacity,visibility] duration-200 motion-reduce:transition-none md:hidden",
            sidebarOpen ? "visible opacity-100" : "invisible opacity-0",
          )}
          onClick={() => setSidebarOpen(false)}
        />
        <div
          id="studio-sidebar"
          data-parked={parked && !sidebarOpen}
          className={`studio-sidebar h-full shrink-0 ${sidebarOpen ? "studio-sidebar-open" : ""}`}
        >
          <AppSidebar
            collapsed={desktop && sidebarCollapsed}
            animate={animateSidebar}
            onCollapsedChange={(collapsed) => {
              setAnimateSidebar(true);
              setSidebarOpen(false);
              useStudio
                .getState()
                .setPref("studio-sidebar-collapsed", String(collapsed));
            }}
          />
          <Button
            aria-label="Close projects"
            size="icon-sm"
            variant="ghost"
            className="absolute end-2 top-3 md:hidden"
            tabIndex={sidebarOpen ? 0 : -1}
            onClick={() => setSidebarOpen(false)}
          >
            <XIcon />
          </Button>
        </div>
        <main className="flex min-w-0 flex-1 flex-col">
          <div className="flex h-10 shrink-0 items-center border-b px-2 md:hidden">
            <Button
              size="sm"
              variant="ghost-muted"
              aria-controls="studio-sidebar"
              aria-expanded={sidebarOpen}
              onClick={() => setSidebarOpen(true)}
            >
              <MenuIcon /> Projects
            </Button>
          </div>
          <Outlet />
        </main>
      </div>
      {installing && (
        <div
          role="status"
          className="fixed inset-0 z-[300] grid place-items-center bg-background/90"
        >
          <div className="flex items-center gap-3">
            <Spinner /> Installing update. Studio will restart…
          </div>
        </div>
      )}
      {!installing && (!completedSetup || reopenSetup) && (
        <Suspense fallback={null}>
          <Onboarding open onClose={() => setReopenSetup(false)} />
        </Suspense>
      )}
      <ConfirmDialogHost />
      <Toaster />
    </TooltipProvider>
  );
}
