import { useState, useCallback } from "react";
import type { GraphIssue } from "../core/schema";

interface WarningBannerProps {
  issues: GraphIssue[];
}

function buildCopyText(issues: GraphIssue[]): string {
  const lines = [
    "The following issues were found in your knowledge-graph.json.",
    "These are LLM generation errors — not a system bug.",
    "You can ask your agent to fix these specific issues in the knowledge-graph.json file:",
    "",
  ];

  // Auto-corrected first, then dropped
  const sorted = [...issues].sort((a, b) => {
    const order: Record<string, number> = { "auto-corrected": 0, dropped: 1, fatal: 2 };
    return (order[a.level] ?? 2) - (order[b.level] ?? 2);
  });

  for (const issue of sorted) {
    const label =
      issue.level === "auto-corrected"
        ? "Auto-corrected"
        : issue.level === "dropped"
          ? "Dropped"
          : "Fatal";
    lines.push(`[${label}] ${issue.message}`);
  }

  return lines.join("\n");
}

export default function WarningBanner({ issues }: WarningBannerProps) {
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);

  const autoCorrected = issues.filter((i) => i.level === "auto-corrected");
  const dropped = issues.filter((i) => i.level === "dropped");

  // Build summary text — only mention counts > 0
  const parts: string[] = [];
  if (autoCorrected.length > 0) {
    parts.push(`${autoCorrected.length} auto-correction${autoCorrected.length !== 1 ? "s" : ""}`);
  }
  if (dropped.length > 0) {
    parts.push(`${dropped.length} dropped item${dropped.length !== 1 ? "s" : ""}`);
  }
  const summary = `Knowledge graph loaded with ${parts.join(" and ")}`;

  const handleCopy = useCallback(async () => {
    const text = buildCopyText(issues);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      console.warn("Clipboard write failed — copy text manually from the expanded issue list");
    }
  }, [issues]);

  if (issues.length === 0) return null;

  return (
    <div className="bg-amber-900/20 border-b border-amber-700 text-amber-200 text-sm">
      {/* Collapsed summary row */}
      <button
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((prev) => !prev)}
        className="w-full flex items-center gap-2 px-5 py-3 text-left hover:bg-amber-900/10 transition-colors"
      >
        {/* Chevron icon */}
        <svg
          className={`w-4 h-4 shrink-0 text-amber-400 transition-transform duration-200 ${
            expanded ? "rotate-90" : ""
          }`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M9 5l7 7-7 7"
          />
        </svg>

        {/* Warning icon */}
        <svg
          className="w-4 h-4 shrink-0 text-amber-400"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4.5c-.77-.833-2.694-.833-3.464 0L3.34 16.5c-.77.833.192 2.5 1.732 2.5z"
          />
        </svg>

        <span className="flex-1">{summary}</span>

        <span className="text-amber-400/60 text-xs shrink-0">
          {expanded ? "click to collapse" : "click to expand"}
        </span>
      </button>

      {/* Expanded detail panel */}
      {expanded && (
        <div className="px-5 pb-4">
          {/* Issue list */}
          <div className="space-y-1 mb-3">
            {/* Auto-corrected issues */}
            {autoCorrected.length > 0 && (
              <div>
                <h4 className="text-xs font-semibold uppercase tracking-wider text-amber-400 mb-1">
                  Auto-corrected ({autoCorrected.length})
                </h4>
                {autoCorrected.map((issue, i) => (
                  <div key={`ac-${i}`} className="flex items-start gap-2 py-0.5 pl-2 text-amber-200/80">
                    <span className="text-amber-400 shrink-0 mt-0.5">
                      <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                      </svg>
                    </span>
                    <span className="text-xs">{issue.message}</span>
                  </div>
                ))}
              </div>
            )}

            {/* Dropped issues */}
            {dropped.length > 0 && (
              <div className={autoCorrected.length > 0 ? "mt-2" : ""}>
                <h4 className="text-xs font-semibold uppercase tracking-wider text-orange-400 mb-1">
                  Dropped ({dropped.length})
                </h4>
                {dropped.map((issue, i) => (
                  <div key={`dr-${i}`} className="flex items-start gap-2 py-0.5 pl-2 text-orange-300/80">
                    <span className="text-orange-400 shrink-0 mt-0.5">
                      <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                      </svg>
                    </span>
                    <span className="text-xs">{issue.message}</span>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Footer with copy button and actionable message */}
          <div className="flex items-center justify-between pt-2 border-t border-amber-700/50">
            <p className="text-xs text-amber-200/60">
              Copy these issues and ask your agent to fix them in knowledge-graph.json
            </p>
            <button
              type="button"
              onClick={handleCopy}
              className="flex items-center gap-1.5 px-3 py-1 rounded text-xs font-medium bg-amber-800/40 text-amber-200 hover:bg-amber-800/60 transition-colors shrink-0 ml-4"
            >
              {copied ? (
                <>
                  <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                  </svg>
                  Copied!
                </>
              ) : (
                <>
                  <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                    />
                  </svg>
                  Copy Issues
                </>
              )}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
