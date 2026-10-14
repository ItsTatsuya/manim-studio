import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from "@tanstack/react-router";

import { AppShell } from "~/components/AppShell";
import { HomeRedirect } from "~/components/HomeRedirect";
import { ProjectView } from "~/components/ProjectView";
import { RendersView } from "~/components/RendersView";
import { SettingsLayout } from "~/components/settings/SettingsLayout";
import { GeneralSettings } from "~/components/settings/GeneralSettings";
import { ProvidersSettings } from "~/components/settings/ProvidersSettings";
import { NarrationSettingsPage } from "~/components/settings/NarrationSettingsPage";
import { AboutSettings } from "~/components/settings/AboutSettings";

const rootRoute = createRootRoute({ component: AppShell });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: HomeRedirect,
});

export interface ProjectSearch {
  thread?: string;
}

export const projectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects/$projectId",
  validateSearch: (search: Record<string, unknown>): ProjectSearch =>
    typeof search.thread === "string" && /^[0-9a-f]{32}$/.test(search.thread)
      ? { thread: search.thread }
      : {},
  component: ProjectView,
});

const rendersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/renders",
  component: RendersView,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: SettingsLayout,
});

const settingsIndexRoute = createRoute({
  getParentRoute: () => settingsRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/settings/providers" });
  },
});

const settingsChildren = [
  createRoute({
    getParentRoute: () => settingsRoute,
    path: "general",
    component: GeneralSettings,
  }),
  createRoute({
    getParentRoute: () => settingsRoute,
    path: "providers",
    component: ProvidersSettings,
  }),
  createRoute({
    getParentRoute: () => settingsRoute,
    path: "narration",
    component: NarrationSettingsPage,
  }),
  createRoute({
    getParentRoute: () => settingsRoute,
    path: "about",
    component: AboutSettings,
  }),
];

const routeTree = rootRoute.addChildren([
  indexRoute,
  projectRoute,
  rendersRoute,
  settingsRoute.addChildren([settingsIndexRoute, ...settingsChildren]),
]);

// The desktop window loads the app from one page, so hash history keeps reloads safe.
export const router = createRouter({
  routeTree,
  history: createHashHistory(),
  defaultPreload: false,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
