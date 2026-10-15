import { ExternalLinkIcon } from "lucide-react";

import { cn } from "~/lib/utils";

/**
 * What changed in a release: an optional summary and a bulleted list. Long
 * lists show the first few and link to GitHub for the rest. The text is
 * rendered as plain text; the server has already stripped markup.
 */
export function ReleaseNotes({
  summary,
  items = [],
  url,
  limit = 8,
  className,
}: {
  summary?: string;
  /** Absent when talking to a server that predates release notes. */
  items?: string[];
  url: string;
  limit?: number;
  className?: string;
}) {
  const shown = items.slice(0, limit);
  const more = items.length - shown.length;
  return (
    <div className={cn("text-sm", className)}>
      {summary && (
        <p className="mb-3 text-muted-foreground leading-relaxed">{summary}</p>
      )}
      {shown.length > 0 ? (
        <>
          <h3 className="mb-2 font-semibold">What's changed</h3>
          <ul className="max-h-64 list-disc space-y-2 overflow-y-auto ps-5 pe-1 text-foreground/90 leading-snug marker:text-muted-foreground">
            {shown.map((item, index) => (
              <li key={index} className="break-words ps-0.5">
                {item}
              </li>
            ))}
          </ul>
        </>
      ) : (
        !summary && (
          <p className="text-muted-foreground">
            The release notes are on GitHub.
          </p>
        )
      )}
      {(more > 0 || (!shown.length && !summary)) && (
        <a
          href={url}
          target="_blank"
          rel="noreferrer"
          className="mt-3 inline-flex items-center gap-1.5 text-muted-foreground text-xs hover:text-foreground hover:underline"
        >
          {more > 0
            ? `${more} more ${more === 1 ? "change" : "changes"} on GitHub`
            : "View on GitHub"}
          <ExternalLinkIcon className="size-3" />
        </a>
      )}
    </div>
  );
}
