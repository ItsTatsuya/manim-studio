import {
  BrainIcon,
  ChevronRightIcon,
  CircleAlertIcon,
  GitForkIcon,
  FilePenIcon,
  FilePlusIcon,
  FileMinusIcon,
  ListChecksIcon,
  SquareCheckIcon,
  SquareIcon,
  TerminalIcon,
  WrenchIcon,
  XCircleIcon,
} from "lucide-react";
import {
  lazy,
  memo,
  Suspense,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useNavigate } from "@tanstack/react-router";

import { api, type Thread, type ThreadItem, type Turn } from "~/lib/api";
import { effortInfo, modelName } from "~/lib/models";
import { useStudio } from "~/state/studio";
import { cn, formatDuration } from "~/lib/utils";
import type { PanelTab } from "../panel/RightPanel";
import { Spinner } from "../ui/input";
import { toast } from "../ui/toast";
import { useElapsed } from "./useElapsed";

const Markdown = lazy(() => import("./Markdown"));

const WORK_KINDS = new Set([
  "reasoning",
  "command",
  "file_change",
  "tool",
  "plan",
]);

type Block =
  | { type: "item"; item: ThreadItem }
  | { type: "work"; id: string; items: ThreadItem[] };

function groupTurnItems(items: ThreadItem[]): Block[] {
  const blocks: Block[] = [];
  for (const item of items) {
    const last = blocks.at(-1);
    if (WORK_KINDS.has(item.kind)) {
      if (last?.type === "work") last.items.push(item);
      else blocks.push({ type: "work", id: item.id, items: [item] });
    } else blocks.push({ type: "item", item });
  }
  return blocks;
}

export function Timeline({
  thread,
  onShowPanel,
}: {
  thread: Thread;
  onShowPanel: (tab: PanelTab) => void;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const turns = useMemo(() => {
    const byTurn = new Map<string, ThreadItem[]>();
    for (const item of thread.items) {
      const list = byTurn.get(item.turn) ?? [];
      list.push(item);
      byTurn.set(item.turn, list);
    }
    return thread.turns.map((turn) => ({
      turn,
      blocks: groupTurnItems(byTurn.get(turn.id) ?? []),
    }));
  }, [thread.items, thread.turns]);

  useLayoutEffect(() => {
    const element = scroller.current;
    if (element && pinned.current) element.scrollTop = element.scrollHeight;
  });

  useEffect(() => {
    pinned.current = true;
  }, [thread.id]);

  return (
    <div
      ref={scroller}
      className="min-h-0 flex-1 overflow-y-auto"
      onScroll={(event) => {
        const element = event.currentTarget;
        pinned.current =
          element.scrollHeight - element.scrollTop - element.clientHeight < 80;
      }}
    >
      <div
        className="mx-auto flex w-full max-w-[46rem] flex-col gap-5 px-4 pt-6 pb-4"
        role="log"
        aria-label="Conversation"
      >
        {thread.forked_from && (
          <div className="flex items-center gap-1.5 text-muted-foreground text-xs">
            <GitForkIcon className="size-3" />
            Forked from {thread.forked_from.title}. The earlier conversation is
            included as context.
          </div>
        )}
        {turns.map(({ turn, blocks }) => (
          <TurnView
            key={turn.id}
            turn={turn}
            blocks={blocks}
            project={thread.project}
            threadId={thread.id}
            onShowPanel={onShowPanel}
          />
        ))}
      </div>
    </div>
  );
}

function TurnView({
  turn,
  blocks,
  project,
  threadId,
  onShowPanel,
}: {
  turn: Turn;
  blocks: Block[];
  project: string;
  threadId: string;
  onShowPanel: (tab: PanelTab) => void;
}) {
  const running = turn.status === "running";
  const providers = useStudio((state) => state.agents?.providers);
  const lastWork = blocks.findLastIndex((block) => block.type === "work");
  const hasReply = blocks.some(
    (block) => block.type === "item" && block.item.kind === "assistant",
  );
  return (
    <div className="flex flex-col gap-3">
      {blocks.map((block, index) =>
        block.type === "work" ? (
          <WorkGroup
            key={block.id}
            items={block.items}
            live={running && index === lastWork}
          />
        ) : (
          <ItemView key={block.item.id} item={block.item} />
        ),
      )}
      {running && (
        <WorkingIndicator
          started={turn.started}
          quiet={hasReply}
          detail={`${modelName(providers, turn.provider, turn.model)} · ${effortInfo(providers, turn.provider, turn.model, turn.effort).label}`}
        />
      )}
      {!running && (
        <TurnFooter
          turn={turn}
          project={project}
          threadId={threadId}
          onShowPanel={onShowPanel}
        />
      )}
    </div>
  );
}

function WorkingIndicator({
  started,
  quiet,
  detail,
}: {
  started: string;
  quiet: boolean;
  detail: string;
}) {
  const elapsed = useElapsed(started);
  return (
    <div className="flex items-center gap-2 text-[13px]" role="status">
      <Spinner className="size-3.5 text-muted-foreground" />
      <span className="shimmer-text font-medium">
        {quiet ? "Still working" : "Working"}
      </span>
      <span className="text-muted-foreground text-xs tabular-nums">
        {formatDuration(elapsed)}
      </span>
      <span className="text-muted-foreground/70 text-xs">· {detail}</span>
    </div>
  );
}

function TurnFooter({
  turn,
  project,
  threadId,
  onShowPanel,
}: {
  turn: Turn;
  project: string;
  threadId: string;
  onShowPanel: (tab: PanelTab) => void;
}) {
  const providers = useStudio((state) => state.agents?.providers);
  const navigate = useNavigate();
  const [forking, setForking] = useState(false);
  if (!turn.finished) return null;
  const seconds =
    (new Date(turn.finished).getTime() - new Date(turn.started).getTime()) /
    1000;
  const effort = effortInfo(providers, turn.provider, turn.model, turn.effort);
  const fork = async () => {
    setForking(true);
    try {
      const thread = await api.forkThread(project, threadId, { turn: turn.id });
      useStudio.getState().upsertThreadSummary(thread);
      useStudio.setState((state) => ({
        details: { ...state.details, [thread.id]: thread },
      }));
      void navigate({
        to: "/projects/$projectId",
        params: { projectId: project },
        search: { thread: thread.id },
      });
    } catch (error) {
      toast.error("Couldn't fork the chat", (error as Error).message);
    } finally {
      setForking(false);
    }
  };
  return (
    <div className="flex flex-wrap items-center gap-1.5 text-muted-foreground text-xs">
      <span
        title={`${modelName(providers, turn.provider, turn.model)} · reasoning effort: ${effort.label}`}
      >
        {modelName(providers, turn.provider, turn.model)} · {effort.label}
      </span>
      <span className="text-muted-foreground/50">·</span>
      <span>
        {turn.status === "interrupted" ? "Stopped after" : "Worked for"}{" "}
        {formatDuration(seconds)}
      </span>
      {turn.changed.length > 0 && (
        <span className="text-muted-foreground/50">·</span>
      )}
      {turn.changed.slice(0, 6).map((file) => (
        <button
          key={file}
          type="button"
          onClick={() =>
            onShowPanel(
              file === "script.py"
                ? "script"
                : file === "narration.txt"
                  ? "narration"
                  : "assets",
            )
          }
          className="rounded-md border bg-card px-1.5 py-0.5 font-mono text-[11px] text-foreground/80 hover:bg-accent"
        >
          {file}
        </button>
      ))}
      {turn.changed.length > 6 && <span>+{turn.changed.length - 6} more</span>}
      <button
        type="button"
        disabled={forking}
        onClick={() => void fork()}
        className="ms-auto flex items-center gap-1 rounded-md px-1.5 py-0.5 hover:bg-accent hover:text-foreground"
      >
        <GitForkIcon className="size-3" /> Fork from here
      </button>
    </div>
  );
}

const ItemView = memo(function ItemView({ item }: { item: ThreadItem }) {
  if (item.kind === "user")
    return (
      <div className="flex justify-end">
        <div className="max-w-[85%] whitespace-pre-wrap break-words rounded-2xl rounded-br-md bg-secondary px-3.5 py-2 text-[14px] leading-relaxed">
          {item.text}
        </div>
      </div>
    );
  if (item.kind === "assistant")
    return (
      <div className="chat-markdown text-[14px]">
        <Suspense fallback={<p className="whitespace-pre-wrap">{item.text}</p>}>
          <Markdown>{item.text || ""}</Markdown>
        </Suspense>
        {item.streaming && (
          <span className="ms-0.5 inline-block h-4 w-1.5 animate-pulse rounded-sm bg-foreground/60 align-middle" />
        )}
      </div>
    );
  if (item.kind === "error")
    return (
      <div
        role="alert"
        className="flex gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-[13px] text-destructive-foreground"
      >
        <CircleAlertIcon className="mt-0.5 size-4 shrink-0" />
        <div className="min-w-0 flex-1">
          {item.title && item.code ? (
            <>
              <div className="flex flex-wrap items-center gap-2 font-medium">
                {item.title}
                <code className="rounded bg-destructive/20 px-1.5 py-0.5 font-mono text-[10.5px] uppercase tracking-wide">
                  {item.code}
                  {item.http_status ? ` · ${item.http_status}` : ""}
                </code>
              </div>
              {item.hint && <p className="mt-1">{item.hint}</p>}
              {item.reset && (
                <p className="mt-1 font-medium">Resets: {item.reset}</p>
              )}
              {item.text && (
                <details className="mt-1.5 opacity-80">
                  <summary className="cursor-pointer text-xs">
                    Provider message
                  </summary>
                  <div className="mt-1 whitespace-pre-wrap break-words text-xs">
                    {item.text}
                  </div>
                </details>
              )}
            </>
          ) : (
            <div className="whitespace-pre-wrap break-words">{item.text}</div>
          )}
        </div>
      </div>
    );
  if (item.kind === "notice")
    return <div className="text-muted-foreground text-xs">{item.text}</div>;
  return null;
});

function workSummary(items: ThreadItem[]) {
  const commands = items.filter((item) => item.kind === "command").length;
  const files = new Set(
    items.flatMap((item) => item.files?.map((file) => file.path) ?? []),
  ).size;
  const parts = [];
  if (files) parts.push(`edited ${files} file${files === 1 ? "" : "s"}`);
  if (commands)
    parts.push(`ran ${commands} command${commands === 1 ? "" : "s"}`);
  const other =
    items.length -
    commands -
    items.filter((item) => item.kind === "file_change").length;
  if (!parts.length || other > 0)
    parts.push(`${items.length} step${items.length === 1 ? "" : "s"}`);
  const text = parts.join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function WorkGroup({ items, live }: { items: ThreadItem[]; live: boolean }) {
  const [open, setOpen] = useState<boolean | null>(null);
  const expanded = open ?? live;
  const failed = items.some((item) => item.status === "failed");
  return (
    <div className="rounded-xl border bg-card/40">
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setOpen(!expanded)}
        className="flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left text-muted-foreground text-xs hover:text-foreground"
      >
        <ChevronRightIcon
          className={cn(
            "size-3.5 transition-transform",
            expanded && "rotate-90",
          )}
        />
        <span className="font-medium">{workSummary(items)}</span>
        {failed && (
          <XCircleIcon
            className="size-3.5 text-destructive-foreground"
            aria-label="A step failed"
          />
        )}
        {live && <Spinner className="ms-auto size-3" />}
      </button>
      {expanded && (
        <div className="flex flex-col gap-0.5 border-t px-2 py-1.5">
          {items.map((item) => (
            <WorkRow key={item.id} item={item} />
          ))}
        </div>
      )}
    </div>
  );
}

function StatusIcon({ status }: { status?: string }) {
  if (status === "in_progress") return <Spinner className="size-3" />;
  if (status === "failed")
    return <XCircleIcon className="size-3.5 text-destructive-foreground" />;
  return null;
}

function WorkRow({ item }: { item: ThreadItem }) {
  const [open, setOpen] = useState(false);
  if (item.kind === "plan")
    return (
      <div className="px-1.5 py-1">
        <div className="mb-1 flex items-center gap-2 text-muted-foreground text-xs">
          <ListChecksIcon className="size-3.5" /> Plan
        </div>
        <ul className="space-y-0.5 ps-5 text-[13px]">
          {item.entries?.map((entry, index) => (
            <li
              key={index}
              className={cn(
                "flex items-start gap-1.5",
                entry.done && "text-muted-foreground line-through",
              )}
            >
              {entry.done ? (
                <SquareCheckIcon className="mt-0.5 size-3.5 shrink-0" />
              ) : (
                <SquareIcon className="mt-0.5 size-3.5 shrink-0" />
              )}
              {entry.text}
            </li>
          ))}
        </ul>
      </div>
    );
  if (item.kind === "file_change")
    return (
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 px-1.5 py-1 text-[13px]">
        {item.files?.map((file) => (
          <span key={file.path} className="flex items-center gap-1.5">
            {file.kind === "add" ? (
              <FilePlusIcon className="size-3.5 text-success-foreground" />
            ) : file.kind === "delete" ? (
              <FileMinusIcon className="size-3.5 text-destructive-foreground" />
            ) : (
              <FilePenIcon className="size-3.5 text-muted-foreground" />
            )}
            <span className="text-muted-foreground">
              {file.kind === "add"
                ? "Created"
                : file.kind === "delete"
                  ? "Removed"
                  : "Edited"}
            </span>
            <span className="font-mono text-xs">{file.path}</span>
          </span>
        ))}
        <StatusIcon status={item.status} />
      </div>
    );
  const icon =
    item.kind === "command" ? (
      <TerminalIcon className="size-3.5 shrink-0" />
    ) : item.kind === "reasoning" ? (
      <BrainIcon className="size-3.5 shrink-0" />
    ) : (
      <WrenchIcon className="size-3.5 shrink-0" />
    );
  const label =
    item.kind === "reasoning" ? item.text?.split("\n")[0] : item.title;
  const detail = item.kind === "reasoning" ? item.text : item.detail;
  return (
    <div>
      <button
        type="button"
        disabled={!detail}
        onClick={() => setOpen(!open)}
        aria-expanded={detail ? open : undefined}
        className="flex w-full min-w-0 items-center gap-2 rounded-md px-1.5 py-1 text-left text-[13px] text-muted-foreground enabled:hover:bg-accent enabled:hover:text-foreground"
      >
        {icon}
        <span
          className={cn(
            "min-w-0 flex-1 truncate",
            item.kind === "command" && "font-mono text-xs",
          )}
        >
          {label}
        </span>
        <StatusIcon status={item.status} />
      </button>
      {open && detail && (
        <pre className="mx-1.5 mt-0.5 mb-1.5 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-md border bg-code p-2 font-mono text-[11.5px] text-muted-foreground leading-relaxed">
          {detail}
        </pre>
      )}
    </div>
  );
}
