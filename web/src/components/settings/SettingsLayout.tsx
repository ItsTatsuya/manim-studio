import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { cn } from "~/lib/utils";
import { WorkspaceBackButton } from "../WorkspaceBackButton";

const pages = [
  { to: "/settings/providers", label: "Providers" },
  { to: "/settings/general", label: "General" },
  { to: "/settings/narration", label: "Narration" },
  { to: "/settings/about", label: "About & updates" },
] as const;

/**
 * Settings pages live in the sidebar on desktop (see AppSidebar). Below the
 * drawer breakpoint the sidebar is closed, so the same pages show here.
 */
export function SettingsLayout() {
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const current = pages.find((page) => pathname.startsWith(page.to));
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-[52px] shrink-0 items-center gap-1.5 border-b px-4 text-sm">
        <WorkspaceBackButton className="me-1.5 md:hidden" />
        <span className="text-muted-foreground">Settings</span>
        <span className="text-muted-foreground/60">/</span>
        <h1 className="truncate font-medium">{current?.label ?? "Settings"}</h1>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto [scrollbar-gutter:stable]">
        <div className="mx-auto max-w-3xl px-6 py-6">
          <nav
            aria-label="Settings"
            className="mb-6 flex flex-wrap gap-1 rounded-lg bg-muted p-1 md:hidden"
          >
            {pages.map((page) => (
              <Link
                key={page.to}
                to={page.to}
                activeOptions={{ exact: true, includeSearch: false }}
                className="studio-tab inline-flex h-8 items-center rounded-md px-3 text-sm text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              >
                {page.label}
              </Link>
            ))}
          </nav>
          <Outlet />
        </div>
      </div>
    </div>
  );
}

/** A titled group of settings: a muted heading above one bordered card of rows. */
export function SettingsSection({
  title,
  description,
  action,
  children,
  className,
}: {
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("mb-8", className)}>
      <div className="mb-2 flex min-h-8 items-end justify-between gap-4 px-1">
        <div className="min-w-0">
          <h2 className="font-medium text-muted-foreground text-sm">{title}</h2>
          {description && (
            <p className="mt-0.5 text-muted-foreground text-xs leading-relaxed">
              {description}
            </p>
          )}
        </div>
        {action}
      </div>
      <div className="divide-y overflow-hidden rounded-xl border bg-card">
        {children}
      </div>
    </section>
  );
}

/** One setting: title and description on the left, its control on the right. */
export function SettingsRow({
  title,
  description,
  children,
  className,
}: {
  title: string;
  description?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex items-center justify-between gap-6 px-4 py-3.5",
        className,
      )}
    >
      <div className="min-w-0">
        <h3 className="font-medium text-sm">{title}</h3>
        {description && (
          <p className="mt-0.5 text-muted-foreground text-xs leading-relaxed">
            {description}
          </p>
        )}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}
