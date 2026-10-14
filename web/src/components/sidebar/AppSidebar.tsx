import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import {
  ChevronRightIcon,
  ClapperboardIcon,
  EllipsisIcon,
  GitForkIcon,
  FolderIcon,
  FolderOpenIcon,
  FolderPlusIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  PencilIcon,
  SettingsIcon,
  SquarePenIcon,
  Trash2Icon,
  UploadIcon,
  WifiOffIcon,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { api, type ProjectSummary, type ThreadSummary } from "~/lib/api";
import { cn, relativeTime } from "~/lib/utils";
import { useStudio } from "~/state/studio";
import { ImportProjectDialog } from "../ImportProjectDialog";
import { NewProjectDialog } from "../NewProjectDialog";
import { RenameDialog } from "../RenameDialog";
import { StudioMark } from "../icons";
import { Button } from "../ui/button";
import { confirmAction } from "../ui/dialog";
import { Spinner, StatusDot } from "../ui/input";
import {
  Menu,
  MenuItem,
  MenuPopup,
  MenuSeparator,
  MenuTrigger,
} from "../ui/menu";
import { toast } from "../ui/toast";
import { UpdateNotice } from "./UpdateNotice";
import { Tooltip } from "../ui/tooltip";

function useExpansion() {
  const raw = useStudio((state) => state.prefs["manim-project-expanded"]);
  const expanded = useMemo<Record<string, boolean>>(() => {
    try {
      const value = JSON.parse(raw || "{}");
      return value && typeof value === "object" ? value : {};
    } catch {
      return {};
    }
  }, [raw]);
  const setExpanded = (id: string, open: boolean) =>
    useStudio
      .getState()
      .setPref(
        "manim-project-expanded",
        JSON.stringify({ ...expanded, [id]: open }),
      );
  return [expanded, setExpanded] as const;
}

export function AppSidebar({
  collapsed,
  onCollapsedChange,
}: {
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
}) {
  const projects = useStudio((state) => state.projects);
  const connected = useStudio((state) => state.connected);
  const location = useRouterState({ select: (state) => state.location });
  const currentProject = location.pathname.match(
    /^\/projects\/([0-9a-f]{32})/,
  )?.[1];
  const currentThread = (location.search as { thread?: string }).thread;
  const [expanded, setExpanded] = useExpansion();
  const [creating, setCreating] = useState(false);
  const [importing, setImporting] = useState(false);

  return (
    <aside
      aria-label="Studio sidebar"
      className={cn(
        "flex h-full shrink-0 flex-col border-sidebar-border border-r bg-sidebar text-sidebar-foreground",
        collapsed ? "w-14" : "w-[264px]",
      )}
    >
      <div
        className={cn(
          "flex h-[52px] shrink-0 items-center gap-2",
          collapsed ? "justify-center px-2" : "px-4",
        )}
      >
        {!collapsed && (
          <>
            <StudioMark className="size-[22px]" />
            <span className="font-semibold text-[15px] tracking-tight">
              Manim Studio
            </span>
          </>
        )}
        <Tooltip
          content={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          side="right"
        >
          <Button
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-expanded={!collapsed}
            size="icon-sm"
            variant="ghost-muted"
            className={cn("hidden md:inline-flex", !collapsed && "ms-auto")}
            onClick={() => onCollapsedChange(!collapsed)}
          >
            {collapsed ? <PanelLeftOpenIcon /> : <PanelLeftCloseIcon />}
          </Button>
        </Tooltip>
      </div>

      <div
        className={cn(
          "flex pt-2 pb-1",
          collapsed
            ? "justify-center px-2"
            : "items-center justify-between px-4",
        )}
      >
        {!collapsed && (
          <span className="font-medium text-[11px] text-sidebar-muted-foreground uppercase tracking-wider">
            Projects
          </span>
        )}
        <div
          className={cn(
            "flex items-center",
            collapsed ? "flex-col gap-2" : "-me-1.5",
          )}
        >
          <Tooltip
            content="Import project"
            side={collapsed ? "right" : "bottom"}
          >
            <Button
              size={collapsed ? "icon" : "icon-xs"}
              variant="ghost-muted"
              aria-label="Import project"
              onClick={() => setImporting(true)}
            >
              <UploadIcon />
            </Button>
          </Tooltip>
          <Tooltip content="New project" side={collapsed ? "right" : "bottom"}>
            <Button
              size={collapsed ? "icon" : "icon-xs"}
              variant="ghost-muted"
              aria-label="New project"
              onClick={() => setCreating(true)}
            >
              <FolderPlusIcon />
            </Button>
          </Tooltip>
        </div>
      </div>

      <nav
        aria-label="Projects"
        className="min-h-0 flex-1 overflow-y-auto px-2 pb-2"
      >
        {projects.map((project) =>
          collapsed ? (
            <Tooltip key={project.id} content={project.name} side="right">
              <Link
                to="/projects/$projectId"
                params={{ projectId: project.id }}
                search={{}}
                aria-label={project.name}
                aria-current={
                  project.id === currentProject ? "page" : undefined
                }
                className={cn(
                  "mb-1 flex h-9 w-full items-center justify-center rounded-md text-sidebar-muted-foreground outline-none hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-ring",
                  project.id === currentProject &&
                    "bg-sidebar-row-active text-sidebar-foreground",
                )}
              >
                {project.id === currentProject ? (
                  <FolderOpenIcon className="size-4" />
                ) : (
                  <FolderIcon className="size-4" />
                )}
              </Link>
            </Tooltip>
          ) : (
            <ProjectGroup
              key={project.id}
              project={project}
              active={project.id === currentProject}
              activeThread={
                project.id === currentProject ? currentThread : undefined
              }
              expanded={expanded[project.id] ?? project.id === currentProject}
              onExpandedChange={(open) => setExpanded(project.id, open)}
            />
          ),
        )}
        {!projects.length && !collapsed && (
          <p className="px-2 py-3 text-sidebar-muted-foreground text-xs">
            No projects yet. Create one to start.
          </p>
        )}
      </nav>

      <div className="flex shrink-0 flex-col gap-0.5 border-sidebar-border border-t p-2">
        {!connected && (
          <Tooltip
            content="Reconnecting to Studio…"
            side={collapsed ? "right" : "top"}
          >
            <div
              role="status"
              aria-label="Reconnecting to Studio"
              className={cn(
                "mb-1 flex items-center gap-2 rounded-md bg-warning/10 px-2 py-1.5 text-warning-foreground text-xs",
                collapsed && "justify-center",
              )}
            >
              <WifiOffIcon className="size-3.5" />{" "}
              {!collapsed && "Reconnecting to Studio…"}
            </div>
          </Tooltip>
        )}
        <UpdateNotice collapsed={collapsed} />
        <SidebarLink
          to="/renders"
          icon={<ClapperboardIcon />}
          label="My renders"
          collapsed={collapsed}
        />
        <SidebarLink
          to="/settings"
          icon={<SettingsIcon />}
          label="Settings"
          collapsed={collapsed}
        />
      </div>

      <NewProjectDialog open={creating} onOpenChange={setCreating} />
      <ImportProjectDialog open={importing} onOpenChange={setImporting} />
    </aside>
  );
}

function SidebarLink({
  to,
  icon,
  label,
  collapsed = false,
}: {
  to: "/renders" | "/settings";
  icon: React.ReactNode;
  label: string;
  collapsed?: boolean;
}) {
  return (
    <Tooltip content={collapsed ? label : null} side="right">
      <Link
        to={to}
        aria-label={label}
        activeOptions={{ includeSearch: false }}
        className={cn(
          "flex h-8 items-center gap-2.5 rounded-md px-2 text-[13px] text-sidebar-muted-foreground outline-none transition-colors hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-ring data-[status=active]:bg-sidebar-row-active data-[status=active]:text-sidebar-foreground [&_svg]:size-4",
          collapsed && "justify-center",
        )}
      >
        {icon}
        {!collapsed && label}
      </Link>
    </Tooltip>
  );
}

function ProjectGroup({
  project,
  active,
  activeThread,
  expanded,
  onExpandedChange,
}: {
  project: ProjectSummary;
  active: boolean;
  activeThread?: string;
  expanded: boolean;
  onExpandedChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const threads = useStudio((state) => state.threads[project.id]);
  const loadThreads = useStudio((state) => state.loadThreads);
  const [renaming, setRenaming] = useState(false);

  useEffect(() => {
    if (expanded && !threads)
      void loadThreads(project.id).catch(() => undefined);
  }, [expanded, threads, loadThreads, project.id]);

  const running = threads?.some((thread) => thread.status === "running");

  const newChat = () => {
    onExpandedChange(true);
    void navigate({
      to: "/projects/$projectId",
      params: { projectId: project.id },
      search: {},
    });
  };

  const remove = async () => {
    const ok = await confirmAction({
      title: `Delete “${project.name}”?`,
      description:
        "This removes the project's script, narration, assets and chats. Finished videos stay in My renders.",
      confirm: "Delete project",
      destructive: true,
    });
    if (!ok) return;
    try {
      const result = await api.deleteProject(project.id);
      const projects = await useStudio.getState().loadProjects();
      if (active) {
        const next = result.replacement ?? projects[0];
        if (next)
          void navigate({
            to: "/projects/$projectId",
            params: { projectId: next.id },
            search: {},
          });
        else void navigate({ to: "/" });
      }
    } catch (error) {
      toast.error("Couldn't delete the project", (error as Error).message);
    }
  };

  return (
    <div className="mb-0.5">
      <div
        className={cn(
          "group/project relative flex h-8 items-center rounded-md text-[13px] transition-colors hover:bg-sidebar-row-hover",
          active && !activeThread && "bg-sidebar-row-active",
        )}
      >
        <button
          type="button"
          aria-label={
            expanded ? `Collapse ${project.name}` : `Expand ${project.name}`
          }
          aria-expanded={expanded}
          className="flex size-6 shrink-0 items-center justify-center rounded text-sidebar-muted-foreground hover:text-sidebar-foreground"
          onClick={() => onExpandedChange(!expanded)}
        >
          <ChevronRightIcon
            className={cn(
              "size-3.5 transition-transform duration-150",
              expanded && "rotate-90",
            )}
          />
        </button>
        <Link
          to="/projects/$projectId"
          params={{ projectId: project.id }}
          search={{}}
          onClick={() => onExpandedChange(true)}
          className="flex min-w-0 flex-1 items-center gap-2 py-1 pe-14 outline-none"
        >
          {expanded ? (
            <FolderOpenIcon className="size-4 shrink-0 text-sidebar-muted-foreground" />
          ) : (
            <FolderIcon className="size-4 shrink-0 text-sidebar-muted-foreground" />
          )}
          <span
            className={cn(
              "truncate",
              active
                ? "font-medium text-sidebar-foreground"
                : "text-sidebar-foreground/90",
            )}
          >
            {project.name}
          </span>
          {running && !expanded && (
            <Spinner className="size-3 shrink-0 text-primary" />
          )}
        </Link>
        <div className="absolute end-1 flex items-center opacity-0 transition-opacity focus-within:opacity-100 group-hover/project:opacity-100 has-[[data-popup-open]]:opacity-100">
          <Tooltip content="New chat">
            <Button
              size="icon-xs"
              variant="ghost-muted"
              aria-label={`New chat in ${project.name}`}
              onClick={newChat}
            >
              <SquarePenIcon />
            </Button>
          </Tooltip>
          <Menu>
            <MenuTrigger
              render={
                <Button
                  size="icon-xs"
                  variant="ghost-muted"
                  aria-label={`${project.name} actions`}
                />
              }
            >
              <EllipsisIcon />
            </MenuTrigger>
            <MenuPopup align="end">
              <MenuItem onClick={() => setRenaming(true)}>
                <PencilIcon /> Rename
              </MenuItem>
              <MenuItem onClick={() => void api.openProjectFolder(project.id)}>
                <FolderOpenIcon /> Open project folder
              </MenuItem>
              <MenuSeparator />
              <MenuItem destructive onClick={() => void remove()}>
                <Trash2Icon /> Delete project
              </MenuItem>
            </MenuPopup>
          </Menu>
        </div>
      </div>
      {expanded && (
        <div className="relative ms-3 border-sidebar-border border-l ps-1.5">
          {threads === undefined && <div className="h-7" />}
          {threads?.map((thread) => (
            <ThreadRow
              key={thread.id}
              thread={thread}
              active={thread.id === activeThread}
            />
          ))}
          {threads?.length === 0 && (
            <button
              type="button"
              onClick={newChat}
              className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-sidebar-muted-foreground text-xs hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
            >
              <SquarePenIcon className="size-3.5" /> New chat
            </button>
          )}
        </div>
      )}
      <RenameDialog
        open={renaming}
        onOpenChange={setRenaming}
        title="Rename project"
        initial={project.name}
        onSubmit={async (name) => {
          await api.updateProject(project.id, { name });
          await useStudio.getState().loadProjects();
        }}
      />
    </div>
  );
}

function ThreadStatusIcon({ thread }: { thread: ThreadSummary }) {
  if (thread.status === "running")
    return <Spinner className="size-3 text-primary" />;
  if (thread.status === "failed") return <StatusDot tone="error" />;
  return <span className="size-2 shrink-0" />;
}

function ThreadRow({
  thread,
  active,
}: {
  thread: ThreadSummary;
  active: boolean;
}) {
  const navigate = useNavigate();
  const [renaming, setRenaming] = useState(false);
  const fork = async () => {
    try {
      const forked = await api.forkThread(thread.project, thread.id);
      useStudio.getState().upsertThreadSummary(forked);
      useStudio.setState((state) => ({
        details: { ...state.details, [forked.id]: forked },
      }));
      void navigate({
        to: "/projects/$projectId",
        params: { projectId: thread.project },
        search: { thread: forked.id },
      });
    } catch (error) {
      toast.error("Couldn't fork the chat", (error as Error).message);
    }
  };
  const remove = async () => {
    const ok = await confirmAction({
      title: "Delete this chat?",
      description:
        "The conversation is removed. Files the agent created stay in the project.",
      confirm: "Delete chat",
      destructive: true,
    });
    if (!ok) return;
    try {
      await api.deleteThread(thread.project, thread.id);
      useStudio.getState().removeThread(thread.project, thread.id);
      if (active)
        void navigate({
          to: "/projects/$projectId",
          params: { projectId: thread.project },
          search: {},
        });
    } catch (error) {
      toast.error("Couldn't delete the chat", (error as Error).message);
    }
  };
  return (
    <div
      className={cn(
        "group/thread relative flex h-7 items-center rounded-md text-[13px] transition-colors hover:bg-sidebar-row-hover",
        active && "bg-sidebar-row-active",
      )}
    >
      <Link
        to="/projects/$projectId"
        params={{ projectId: thread.project }}
        search={{ thread: thread.id }}
        className="flex min-w-0 flex-1 items-center gap-2 px-2 outline-none"
      >
        <ThreadStatusIcon thread={thread} />
        <span
          className={cn(
            "truncate",
            active ? "text-sidebar-foreground" : "text-sidebar-foreground/80",
          )}
        >
          {thread.title}
        </span>
        <span className="ms-auto shrink-0 ps-1 text-[11px] text-sidebar-muted-foreground group-hover/thread:invisible">
          {relativeTime(thread.updated)}
        </span>
      </Link>
      <div className="absolute end-0.5 opacity-0 group-hover/thread:opacity-100 has-[[data-popup-open]]:opacity-100">
        <Menu>
          <MenuTrigger
            render={
              <Button
                size="icon-xs"
                variant="ghost-muted"
                aria-label={`${thread.title} actions`}
              />
            }
          >
            <EllipsisIcon />
          </MenuTrigger>
          <MenuPopup align="end">
            <MenuItem onClick={() => setRenaming(true)}>
              <PencilIcon /> Rename
            </MenuItem>
            <MenuItem onClick={() => void fork()}>
              <GitForkIcon /> Fork chat
            </MenuItem>
            <MenuItem destructive onClick={() => void remove()}>
              <Trash2Icon /> Delete chat
            </MenuItem>
          </MenuPopup>
        </Menu>
      </div>
      <RenameDialog
        open={renaming}
        onOpenChange={setRenaming}
        title="Rename chat"
        initial={thread.title}
        onSubmit={async (title) => {
          const summary = await api.updateThread(thread.project, thread.id, {
            title,
          });
          useStudio.getState().upsertThreadSummary(summary);
        }}
      />
    </div>
  );
}
