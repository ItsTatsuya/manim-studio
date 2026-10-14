import { Link, Outlet } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { cn } from "~/lib/utils";

const pages = [
  { to: "/settings/providers", label: "Providers" },
  { to: "/settings/general", label: "General" },
  { to: "/settings/narration", label: "Narration" },
  { to: "/settings/about", label: "About & updates" },
] as const;

export function SettingsLayout() {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-[52px] shrink-0 items-center border-b px-6 text-sm font-medium">
        Settings
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto [scrollbar-gutter:stable]">
        <div className="mx-auto max-w-3xl px-6 py-6">
          <nav
            aria-label="Settings"
            className="mb-7 flex flex-wrap gap-1 rounded-lg bg-muted p-1"
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

export function SettingsHeading({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="mb-5">
      <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
      <p className="mt-1 text-sm leading-relaxed text-muted-foreground">
        {children}
      </p>
    </div>
  );
}

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
        "flex items-center justify-between gap-6 border-b py-4",
        className,
      )}
    >
      <div className="min-w-0">
        <h2 className="text-sm font-medium">{title}</h2>
        {description && (
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}
