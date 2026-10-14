import {
  createHashHistory,
  lazyRouteComponent,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from "@tanstack/react-router";

import { AppShell } from "~/components/AppShell";
import { HomeRedirect } from "~/components/HomeRedirect";
import { ProjectView } from "~/components/ProjectView";
import { SettingsLayout } from "~/components/settings/SettingsLayout";

const rootRoute = createRootRoute({ component: AppShell });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: HomeRedirect,
});

interface ProjectSearch {
  thread?: string;
}

const projectRoute = createRoute({
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
  component: lazyRouteComponent(
    () => import("~/components/RendersView"),
    "RendersView",
  ),
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
    component: lazyRouteComponent(
      () => import("~/components/settings/GeneralSettings"),
      "GeneralSettings",
    ),
  }),
  createRoute({
    getParentRoute: () => settingsRoute,
    path: "providers",
    component: lazyRouteComponent(
      () => import("~/components/settings/ProvidersSettings"),
      "ProvidersSettings",
    ),
  }),
  createRoute({
    getParentRoute: () => settingsRoute,
    path: "narration",
    component: lazyRouteComponent(
      () => import("~/components/settings/NarrationSettingsPage"),
      "NarrationSettingsPage",
    ),
  }),
  createRoute({
    getParentRoute: () => settingsRoute,
    path: "about",
    component: lazyRouteComponent(
      () => import("~/components/settings/AboutSettings"),
      "AboutSettings",
    ),
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
