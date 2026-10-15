import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import {
  BotIcon,
  ClapperboardIcon,
  MessageSquareIcon,
  EllipsisIcon,
  GitForkIcon,
  FolderIcon,
  FolderOpenIcon,
  FolderPlusIcon,
  SearchIcon,
  CheckIcon,
  ListFilterIcon,
  FilesIcon,
  InfoIcon,
  MicIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  PencilIcon,
  SettingsIcon,
  SlidersHorizontalIcon,
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
import { BatchVideosDialog } from "../BatchVideosDialog";
import { NewProjectDialog } from "../NewProjectDialog";
import { RenameDialog } from "../RenameDialog";
import { StudioMark } from "../icons";
import { Button } from "../ui/button";
import { confirmAction } from "../ui/dialog";
import { Input, Spinner, StatusDot } from "../ui/input";
import {
  Popover,
  PopoverPopup,
  PopoverTitle,
  PopoverTrigger,
} from "../ui/popover";
import {
  Menu,
  MenuItem,
  MenuPopup,
  MenuSeparator,
  MenuTrigger,
} from "../ui/menu";
import { toast } from "../ui/toast";
import { UpdateButton, UpdateWatcher } from "./UpdateNotice";
import { Tooltip } from "../ui/tooltip";
import { UsageButton } from "./UsageButton";
import { SidebarRail } from "./SidebarRail";
import { WorkspaceBackButton } from "../WorkspaceBackButton";

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

const SETTINGS_PAGES = [
  { to: "/settings/providers", label: "Providers", icon: <BotIcon /> },
  {
    to: "/settings/general",
    label: "General",
    icon: <SlidersHorizontalIcon />,
  },
  { to: "/settings/narration", label: "Narration", icon: <MicIcon /> },
  { to: "/settings/about", label: "About & updates", icon: <InfoIcon /> },
] as const;

// Every icon in the sidebar sits in one column: the row is inset 12px inside the
// 8px gutter, so a 16px icon is centred in the 56px rail. Labels clip and fade,
// so nothing moves sideways when the sidebar opens or closes.
const rowClass =
  "flex h-9 w-full items-center gap-2.5 rounded-md px-3 text-ui text-sidebar-muted-foreground outline-none transition-colors hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-4 [&_svg]:shrink-0";

function RowLabel({
  collapsed,
  className,
  children,
}: {
  collapsed: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <span
      className={cn(
        "min-w-0 whitespace-nowrap transition-opacity duration-150 motion-reduce:transition-none",
        collapsed && "opacity-0",
        className,
      )}
    >
      {children}
    </span>
  );
}

export function AppSidebar({
  collapsed,
  animate = false,
  onCollapsedChange,
}: {
  collapsed: boolean;
  /** Animate the width; set only for a user-initiated open or close. */
  animate?: boolean;
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
  const [batch, setBatch] = useState(false);
  const [query, setQuery] = useState("");
  const [folderQuery, setFolderQuery] = useState("");
  const [selectedFolders, setSelectedFolders] = useState<string[] | null>(null);
  const visibleProjects = projects.filter(
    (project) =>
      (selectedFolders === null || selectedFolders.includes(project.id)) &&
      project.name.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const inSettings = location.pathname.startsWith("/settings");
  const secondaryPage = inSettings || location.pathname === "/renders";

  return (
    <aside
      aria-label="Studio sidebar"
      data-collapsed={collapsed}
      className={cn(
        "flex h-full shrink-0 flex-col overflow-clip border-sidebar-border border-r bg-sidebar text-sidebar-foreground",
        animate &&
          "md:transition-[width] md:duration-200 md:ease-[var(--ease-drawer)] motion-reduce:transition-none",
        collapsed ? "w-14" : "w-[264px]",
      )}
    >
      <div className="flex h-[52px] shrink-0 items-center gap-2 px-[9.5px]">
        <Tooltip
          content={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          side="right"
        >
          <Button
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-expanded={!collapsed}
            size="icon-sm"
            variant="ghost-muted"
            className="hidden shrink-0 md:inline-flex"
            onClick={() => onCollapsedChange(!collapsed)}
          >
            {collapsed ? <PanelLeftOpenIcon /> : <PanelLeftCloseIcon />}
          </Button>
        </Tooltip>
        <span
          aria-hidden={collapsed}
          className={cn(
            "flex min-w-0 items-center gap-2 whitespace-nowrap transition-opacity duration-150 motion-reduce:transition-none",
            collapsed && "opacity-0",
          )}
        >
          <StudioMark className="size-[22px] shrink-0" />
          <span className="font-semibold text-[15px] tracking-tight">
            Manim Studio
          </span>
        </span>
      </div>

      {inSettings ? (
        <nav
          aria-label="Settings"
          className={cn(
            "flex min-h-0 flex-1 flex-col overflow-y-auto overflow-x-hidden pt-2",
            collapsed ? "gap-1 px-2.5" : "gap-0.5 px-2",
          )}
        >
          {SETTINGS_PAGES.map((page) => (
            <SidebarLink
              key={page.to}
              to={page.to}
              icon={page.icon}
              label={page.label}
              collapsed={collapsed}
              exact
            />
          ))}
        </nav>
      ) : (
        <>
          {collapsed && (
            <SidebarRail
              projects={projects}
              currentProject={currentProject}
              currentThread={currentThread}
              onCreate={() => setCreating(true)}
              onImport={() => setImporting(true)}
              onBatch={() => setBatch(true)}
            />
          )}
          {/* Keep text at its expanded width while the outer sidebar clips it. */}
          <div
            className={cn(
              "min-h-0 w-[263px] flex-1 flex-col",
              collapsed ? "hidden" : "flex",
            )}
            aria-hidden={collapsed}
            inert={collapsed}
          >
            <div className="flex shrink-0 flex-col gap-0.5 px-2 pt-1">
              <SidebarAction
                icon={<UploadIcon />}
                label="Import project"
                collapsed={collapsed}
                onClick={() => setImporting(true)}
              />
              <SidebarAction
                icon={<FilesIcon />}
                label="Videos from PDFs"
                collapsed={collapsed}
                onClick={() => setBatch(true)}
              />
            </div>

            <div className="flex h-10 shrink-0 items-center justify-between ps-5 pe-2.5">
              <RowLabel
                collapsed={collapsed}
                className="font-medium text-2xs text-sidebar-muted-foreground uppercase tracking-wider"
              >
                Projects
              </RowLabel>
              {!collapsed && (
                <Popover>
                  <Tooltip content="Select folders">
                    <PopoverTrigger
                      render={
                        <Button
                          size="icon-sm"
                          variant="ghost-muted"
                          aria-label="Select folders"
                        />
                      }
                    >
                      <ListFilterIcon />
                    </PopoverTrigger>
                  </Tooltip>
                  <PopoverPopup
                    side="bottom"
                    align="start"
                    className="w-72 p-2"
                  >
                    <PopoverTitle className="px-2 py-2 text-sm font-medium">
                      Select folders
                    </PopoverTitle>
                    <Input
                      aria-label="Search folders"
                      placeholder="Search folders…"
                      value={folderQuery}
                      onChange={(event) => setFolderQuery(event.target.value)}
                    />
                    <div className="my-2 flex gap-1">
                      <Button
                        size="xs"
                        variant="ghost-muted"
                        onClick={() => setSelectedFolders(null)}
                      >
                        Show all
                      </Button>
                      <Button
                        size="xs"
                        variant="ghost-muted"
                        onClick={() => setSelectedFolders([])}
                      >
                        Clear
                      </Button>
                    </div>
                    <div className="max-h-64 overflow-y-auto">
                      {projects
                        .filter((project) =>
                          project.name
                            .toLowerCase()
                            .includes(folderQuery.trim().toLowerCase()),
                        )
                        .map((project) => {
                          const checked =
                            selectedFolders === null ||
                            selectedFolders.includes(project.id);
                          return (
                            <label
                              key={project.id}
                              className="flex min-h-10 cursor-pointer items-center gap-2 rounded-md px-2 text-sm hover:bg-accent"
                            >
                              <input
                                className="size-4 accent-primary"
                                type="checkbox"
                                checked={checked}
                                onChange={() =>
                                  setSelectedFolders((current) => {
                                    const selected =
                                      current ??
                                      projects.map((item) => item.id);
                                    return checked
                                      ? selected.filter(
                                          (id) => id !== project.id,
                                        )
                                      : [...selected, project.id];
                                  })
                                }
                              />
                              <FolderIcon className="size-4 shrink-0 text-muted-foreground" />
                              <span className="truncate">{project.name}</span>
                              {checked && (
                                <CheckIcon className="ms-auto size-3.5 shrink-0 text-primary" />
                              )}
                            </label>
                          );
                        })}
                    </div>
                  </PopoverPopup>
                </Popover>
              )}
            </div>
            <div
              className={cn(
                "mx-2.5 mb-2 flex h-9 shrink-0 items-center gap-2",
                collapsed && "gap-0",
              )}
            >
              <div
                className={cn(
                  "relative min-w-0 flex-1 transition-opacity",
                  collapsed && "invisible opacity-0",
                )}
                inert={collapsed}
                aria-hidden={collapsed}
              >
                <SearchIcon className="pointer-events-none absolute start-2.5 top-2.5 size-4 text-muted-foreground" />
                <Input
                  className="h-9 ps-9"
                  aria-label="Search projects"
                  placeholder="Search projects…"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                />
              </div>
              <Tooltip content="New project" side={collapsed ? "right" : "top"}>
                <Button
                  size="icon-sm"
                  variant="ghost-muted"
                  aria-label="New project"
                  onClick={() => setCreating(true)}
                >
                  <FolderPlusIcon />
                </Button>
              </Tooltip>
            </div>

            <nav
              aria-label="Projects"
              className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-2 pb-2"
            >
              {(collapsed ? projects : visibleProjects).map((project) => (
                <ProjectGroup
                  key={project.id}
                  project={project}
                  collapsed={collapsed}
                  active={project.id === currentProject}
                  activeThread={
                    project.id === currentProject ? currentThread : undefined
                  }
                  expanded={
                    expanded[project.id] ?? project.id === currentProject
                  }
                  onExpandedChange={(open) => setExpanded(project.id, open)}
                />
              ))}
              {!visibleProjects.length && !collapsed && (
                <p className="px-3 py-3 text-sidebar-muted-foreground text-xs">
                  {projects.length
                    ? "No matching projects. Change your search or folder selection."
                    : "No projects yet. Create one to start."}
                </p>
              )}
            </nav>
          </div>
        </>
      )}

      {/* Fixed layouts keep footer controls from stretching during the width tween. */}
      <div
        className={cn(
          "flex shrink-0 flex-col gap-1 border-sidebar-border border-t p-2",
          collapsed ? "w-[55px]" : "w-[263px]",
        )}
      >
        {!connected && (
          <Tooltip
            content="Reconnecting to Studio…"
            side={collapsed ? "right" : "top"}
          >
            <div
              role="status"
              aria-label="Reconnecting to Studio"
              className={cn(
                "flex h-9 items-center rounded-lg bg-warning/10 text-warning-foreground text-xs",
                collapsed ? "ms-0.5 size-9 justify-center" : "gap-2.5 px-3",
              )}
            >
              <WifiOffIcon className="size-4 shrink-0" />
              {!collapsed && (
                <RowLabel collapsed={false}>Reconnecting to Studio…</RowLabel>
              )}
            </div>
          </Tooltip>
        )}
        {/* A compact icon row; in the rail the same buttons stack in one column. */}
        <div
          className={cn(
            "flex gap-1 px-0.5",
            collapsed ? "flex-col items-start" : "items-center",
          )}
        >
          {secondaryPage ? (
            <WorkspaceBackButton
              compact={collapsed}
              className={collapsed ? undefined : "flex-1"}
            />
          ) : (
            <>
              <FooterIcon
                to="/settings"
                icon={<SettingsIcon />}
                label="Settings"
                collapsed={collapsed}
              />
              <FooterIcon
                to="/renders"
                icon={<ClapperboardIcon />}
                label="My renders"
                collapsed={collapsed}
              />
              <UsageButton collapsed={collapsed} />
            </>
          )}
          <span className={cn(!collapsed && "ms-auto")}>
            <UpdateButton collapsed={collapsed} />
          </span>
        </div>
      </div>

      <UpdateWatcher />
      <NewProjectDialog open={creating} onOpenChange={setCreating} />
      <ImportProjectDialog open={importing} onOpenChange={setImporting} />
      <BatchVideosDialog open={batch} onOpenChange={setBatch} />
    </aside>
  );
}

function SidebarAction({
  icon,
  label,
  collapsed,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  collapsed: boolean;
  onClick: () => void;
}) {
  return (
    <Tooltip content={collapsed ? label : null} side="right">
      <button
        type="button"
        aria-label={label}
        onClick={onClick}
        className={rowClass}
      >
        {icon}
        <RowLabel collapsed={collapsed}>{label}</RowLabel>
      </button>
    </Tooltip>
  );
}

type SidebarRoute =
  "/renders" | "/settings" | (typeof SETTINGS_PAGES)[number]["to"];

function FooterIcon({
  to,
  icon,
  label,
  collapsed,
}: {
  to: "/renders" | "/settings";
  icon: React.ReactNode;
  label: string;
  collapsed: boolean;
}) {
  return (
    <Tooltip content={label} side={collapsed ? "right" : "top"}>
      <Button
        size="icon-sm"
        variant="ghost-muted"
        aria-label={label}
        className="data-[status=active]:bg-accent data-[status=active]:text-foreground"
        render={<Link to={to} activeOptions={{ includeSearch: false }} />}
      >
        {icon}
      </Button>
    </Tooltip>
  );
}

function SidebarLink({
  to,
  icon,
  label,
  collapsed = false,
  exact = false,
}: {
  to: SidebarRoute;
  icon: React.ReactNode;
  label: string;
  collapsed?: boolean;
  exact?: boolean;
}) {
  return (
    <Tooltip content={collapsed ? label : null} side="right">
      <Link
        to={to}
        aria-label={label}
        activeOptions={{ exact, includeSearch: false }}
        className={cn(
          rowClass,
          collapsed && "size-9 justify-center gap-0 p-0",
          "data-[status=active]:bg-sidebar-row-active data-[status=active]:text-sidebar-foreground",
        )}
      >
        {icon}
        {!collapsed && <RowLabel collapsed={false}>{label}</RowLabel>}
      </Link>
    </Tooltip>
  );
}

function ProjectGroup({
  project,
  collapsed,
  active,
  activeThread,
  expanded,
  onExpandedChange,
}: {
  project: ProjectSummary;
  collapsed: boolean;
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

  const open = expanded && !collapsed;
  const highlighted = active && (collapsed || !activeThread);
  return (
    <div className="mb-0.5">
      <div
        className={cn(
          "group/project relative flex h-10 items-center rounded-md text-ui transition-colors hover:bg-sidebar-row-hover",
          highlighted && "bg-sidebar-row-active",
        )}
      >
        <Tooltip content={collapsed ? project.name : null} side="right">
          <Link
            to="/projects/$projectId"
            params={{ projectId: project.id }}
            search={{}}
            aria-expanded={open}
            aria-controls={`project-threads-${project.id}`}
            onClick={(event) => {
              onExpandedChange(!expanded);
              if (active && !collapsed) event.preventDefault();
            }}
            className="flex h-10 min-w-0 flex-1 items-center gap-2.5 rounded-md ps-3 outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {(collapsed ? active : expanded) ? (
              <FolderOpenIcon className="size-4 shrink-0 text-sidebar-muted-foreground" />
            ) : (
              <FolderIcon className="size-4 shrink-0 text-sidebar-muted-foreground" />
            )}
            <span
              className={cn(
                "min-w-0 whitespace-nowrap transition-opacity duration-150 motion-reduce:transition-none",
                collapsed ? "overflow-hidden opacity-0" : "truncate",
                active
                  ? "font-medium text-sidebar-foreground"
                  : "text-sidebar-foreground/90",
              )}
            >
              {project.name}
            </span>
            {running && !expanded && !collapsed && (
              <Spinner className="size-3 shrink-0 text-primary" />
            )}
          </Link>
        </Tooltip>
        {/* Room for the hover actions, which overlay the end of the row. */}
        <span className={cn("w-16 shrink-0", collapsed && "hidden")} />
        <div
          className={cn(
            "absolute end-1 flex items-center opacity-0 transition-opacity focus-within:opacity-100 group-hover/project:opacity-100 has-[[data-popup-open]]:opacity-100",
            collapsed && "hidden",
          )}
        >
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
      {
        // Rows grow and shrink with the grid track instead of popping in. The
        // visibility transition keeps the list focusable only while it is shown.
        <div
          id={`project-threads-${project.id}`}
          className={cn(
            "grid transition-[grid-template-rows,opacity,visibility] duration-200 ease-[var(--ease-drawer)] motion-reduce:transition-none",
            open
              ? "visible grid-rows-[1fr] opacity-100"
              : "invisible grid-rows-[0fr] opacity-0",
          )}
        >
          <div className="min-h-0 overflow-hidden">
            <div className="relative ms-5 border-sidebar-border border-l ps-1.5">
              {threads === undefined && <div className="h-8" />}
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
                  className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-sidebar-muted-foreground text-xs outline-none hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <SquarePenIcon className="size-3.5" /> New chat
                </button>
              )}
            </div>
          </div>
        </div>
      }
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
  // One 14px slot for every state so the title doesn't shift when a run starts.
  return (
    <span className="grid size-3.5 shrink-0 place-items-center">
      {thread.status === "running" ? (
        <Spinner className="size-3.5 text-primary" />
      ) : thread.status === "failed" ? (
        <StatusDot tone="error" />
      ) : (
        <MessageSquareIcon className="size-3.5 text-sidebar-muted-foreground" />
      )}
    </span>
  );
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
        "group/thread relative flex h-8 items-center rounded-md text-xs transition-colors hover:bg-sidebar-row-hover",
        active && "bg-sidebar-row-active",
      )}
    >
      <Link
        to="/projects/$projectId"
        params={{ projectId: thread.project }}
        search={{ thread: thread.id }}
        className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md px-2 outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <ThreadStatusIcon thread={thread} />
        <span
          className={cn(
            "truncate",
            active
              ? "font-medium text-sidebar-foreground"
              : "text-sidebar-foreground/75",
          )}
        >
          {thread.title}
        </span>
        <span className="ms-auto shrink-0 ps-1 text-2xs text-sidebar-muted-foreground group-focus-within/thread:invisible group-hover/thread:invisible">
          {relativeTime(thread.updated)}
        </span>
      </Link>
      <div className="absolute end-0.5 opacity-0 focus-within:opacity-100 group-hover/thread:opacity-100 has-[[data-popup-open]]:opacity-100">
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
