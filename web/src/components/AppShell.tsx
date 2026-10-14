import { Outlet, useRouterState } from "@tanstack/react-router";
import { MenuIcon, XIcon } from "lucide-react";
import { useEffect, useState } from "react";

import { applyTheme, normalizeTheme, watchSystemTheme } from "~/lib/theme";
import { connectEvents, useStudio } from "~/state/studio";
import { AppSidebar } from "./sidebar/AppSidebar";
import { AutoRender } from "./AutoRender";
import { ConfirmDialogHost } from "./ui/dialog";
import { Button } from "./ui/button";
import { Spinner } from "./ui/input";
import { Toaster } from "./ui/toast";
import { TooltipProvider } from "./ui/tooltip";
import { StudioMark } from "./icons";
import { Onboarding } from "./Onboarding";

export function AppShell() {
  const ready = useStudio((state) => state.ready);
  const theme = useStudio((state) =>
    normalizeTheme(state.prefs["manim-theme"]),
  );
  const [failed, setFailed] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const sidebarCollapsed = useStudio(
    (state) => state.prefs["studio-sidebar-collapsed"] === "true",
  );
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
    const desktop = window.matchMedia("(min-width: 768px)");
    const closeMobileSidebar = () => {
      if (desktop.matches) setSidebarOpen(false);
    };
    desktop.addEventListener("change", closeMobileSidebar);
    return () => desktop.removeEventListener("change", closeMobileSidebar);
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
        {sidebarOpen && (
          <button
            aria-label="Close projects"
            className="fixed inset-0 z-30 bg-black/50 md:hidden"
            onClick={() => setSidebarOpen(false)}
          />
        )}
        <div
          id="studio-sidebar"
          className={`studio-sidebar h-full shrink-0 ${sidebarOpen ? "studio-sidebar-open" : ""}`}
        >
          <AppSidebar
            collapsed={sidebarCollapsed && !sidebarOpen}
            onCollapsedChange={(collapsed) => {
              setSidebarOpen(false);
              useStudio
                .getState()
                .setPref("studio-sidebar-collapsed", String(collapsed));
            }}
          />
          {sidebarOpen && (
            <Button
              aria-label="Close projects"
              size="icon-sm"
              variant="ghost"
              className="absolute right-2 top-3 md:hidden"
              onClick={() => setSidebarOpen(false)}
            >
              <XIcon />
            </Button>
          )}
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
      <AutoRender />
      <Onboarding
        open={!installing && (!completedSetup || reopenSetup)}
        onClose={() => setReopenSetup(false)}
      />
      <ConfirmDialogHost />
      <Toaster />
    </TooltipProvider>
  );
}
