import { Link } from "@tanstack/react-router";
import {
  CheckIcon,
  EllipsisIcon,
  FilesIcon,
  FolderIcon,
  FolderOpenIcon,
  FolderPlusIcon,
  SearchIcon,
  UploadIcon,
} from "lucide-react";
import { useState } from "react";

import type { ProjectSummary } from "~/lib/api";
import { cn } from "~/lib/utils";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Menu, MenuItem, MenuPopup, MenuTrigger } from "../ui/menu";
import {
  Popover,
  PopoverPopup,
  PopoverTitle,
  PopoverTrigger,
} from "../ui/popover";
import { Tooltip } from "../ui/tooltip";

/** A few fixed-size controls replace clipped labels and indistinguishable folder icons. */
export function SidebarRail({
  projects,
  currentProject,
  currentThread,
  onCreate,
  onImport,
  onBatch,
}: {
  projects: ProjectSummary[];
  currentProject?: string;
  currentThread?: string;
  onCreate: () => void;
  onImport: () => void;
  onBatch: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const current = projects.find((project) => project.id === currentProject);
  const filtered = projects.filter((project) =>
    project.name.toLowerCase().includes(query.trim().toLowerCase()),
  );
  return (
    <div
      aria-label="Project shortcuts"
      className="flex min-h-0 flex-1 flex-col items-start gap-1 px-2.5 pt-2"
    >
      <Tooltip content="New project" side="right">
        <Button
          size="icon-sm"
          variant="ghost-muted"
          aria-label="New project"
          onClick={onCreate}
        >
          <FolderPlusIcon />
        </Button>
      </Tooltip>
      <Menu>
        <Tooltip content="Import and PDF videos" side="right">
          <MenuTrigger
            render={
              <Button
                size="icon-sm"
                variant="ghost-muted"
                aria-label="Import and PDF videos"
              />
            }
          >
            <EllipsisIcon />
          </MenuTrigger>
        </Tooltip>
        <MenuPopup side="right" align="start">
          <MenuItem onClick={onImport}>
            <UploadIcon />
            Import project
          </MenuItem>
          <MenuItem onClick={onBatch}>
            <FilesIcon />
            Videos from PDFs
          </MenuItem>
        </MenuPopup>
      </Menu>
      <div className="my-2 w-9 border-t border-sidebar-border" />
      <Popover open={open} onOpenChange={setOpen}>
        <Tooltip
          content={current ? `Projects · ${current.name}` : "Projects"}
          side="right"
        >
          <PopoverTrigger
            render={
              <Button
                size="icon-sm"
                variant={current ? "secondary" : "ghost-muted"}
                aria-label="Switch project"
              />
            }
          >
            <FolderOpenIcon />
          </PopoverTrigger>
        </Tooltip>
        <PopoverPopup side="right" align="start" className="w-72 p-2">
          <PopoverTitle className="px-2 py-2 text-sm font-medium">
            Projects
          </PopoverTitle>
          <div className="relative mb-2">
            <SearchIcon className="pointer-events-none absolute start-2.5 top-2.5 size-4 text-muted-foreground" />
            <Input
              autoFocus
              aria-label="Search projects"
              placeholder="Search projects…"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              className="h-9 ps-9"
            />
          </div>
          <nav
            aria-label="Choose a project"
            className="max-h-[min(24rem,calc(100dvh-10rem))] overflow-y-auto"
          >
            {filtered.map((project) => (
              <Link
                key={project.id}
                to="/projects/$projectId"
                params={{ projectId: project.id }}
                search={
                  project.id === currentProject && currentThread
                    ? { thread: currentThread }
                    : {}
                }
                onClick={() => setOpen(false)}
                className={cn(
                  "flex min-h-10 items-center gap-2 rounded-lg px-2 text-sm outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring",
                  project.id === currentProject && "bg-accent font-medium",
                )}
              >
                <FolderIcon className="size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate">{project.name}</span>
                {project.id === currentProject && (
                  <CheckIcon
                    aria-hidden
                    className="size-4 shrink-0 text-primary"
                  />
                )}
              </Link>
            ))}
          </nav>
          {!filtered.length && (
            <p className="px-2 py-4 text-xs text-muted-foreground">
              {projects.length
                ? "No matching projects."
                : "Create a project to get started."}
            </p>
          )}
        </PopoverPopup>
      </Popover>
    </div>
  );
}
