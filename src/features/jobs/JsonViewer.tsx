import { useState } from "react"

import { CodeBlock, CodeToken } from "@/components/data/CodeBlock"
import { Button } from "@/components/ui/button"
import { useCopyToClipboard } from "@/hooks/useCopyToClipboard"

/**
 * Recursive JSON viewer with collapse + copy-to-clipboard.
 *
 * Hand-rolled (rather than pulling `react-json-view` which is pre-React-19)
 * so we can wire syntax colors directly into our `--syntax-*` design
 * tokens. The component is non-virtualized; for the realistic BullMQ
 * payload sizes (a few KB at most), this is well within budget. Pathological
 * 1 MB payloads would need virtualization — out of scope for 2.5.
 *
 * Anatomy:
 *   - One outer card-like block, scrolls vertically if the tree is tall.
 *   - Top-right overlay: `[Copy as JSON]` button → writes
 *     `JSON.stringify(value, null, 2)` to the clipboard via the shared
 *     `useCopyToClipboard` hook.
 *   - Each object / array node renders an expand/collapse twisty.
 *     Default open at depth ≤ `collapseDeeperThan` (default 6); deeper
 *     nodes start collapsed to keep the viewport sane on big payloads.
 *
 * Empty `{}` / `[]` render as a single muted literal — no twisty.
 */
interface JsonViewerProps {
  value: unknown
  copyLabel?: string
  collapseDeeperThan?: number
}

export function JsonViewer({ value, copyLabel = "JSON", collapseDeeperThan = 6 }: JsonViewerProps) {
  const copy = useCopyToClipboard()
  const onCopy = () => copy(JSON.stringify(value, null, 2), copyLabel)

  return (
    <CodeBlock
      label={copyLabel}
      data-testid="json-viewer"
      actions={
        <Button variant="default" size="sm" onClick={onCopy} data-testid="json-viewer-copy">
          Copy as JSON
        </Button>
      }
    >
      <JsonNode
        value={value}
        depth={0}
        collapseDeeperThan={collapseDeeperThan}
        trailingComma={false}
      />
    </CodeBlock>
  )
}

interface NodeProps {
  value: unknown
  depth: number
  collapseDeeperThan: number
  trailingComma: boolean
  /** Object key prefix for this node (omitted at the root). */
  keyName?: string
}

function JsonNode({ value, depth, collapseDeeperThan, trailingComma, keyName }: NodeProps) {
  const keyPrefix = keyName !== undefined ? <KeyToken name={keyName} /> : null
  const trail = trailingComma ? <PunctuationToken text="," /> : null

  if (value === null) {
    return (
      <span>
        {keyPrefix}
        <LiteralToken text="null" />
        {trail}
      </span>
    )
  }
  if (typeof value === "string") {
    return (
      <span>
        {keyPrefix}
        <StringToken value={value} />
        {trail}
      </span>
    )
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return (
      <span>
        {keyPrefix}
        <LiteralToken text={String(value)} />
        {trail}
      </span>
    )
  }
  if (Array.isArray(value)) {
    return (
      <ArrayNode
        items={value}
        depth={depth}
        collapseDeeperThan={collapseDeeperThan}
        trailingComma={trailingComma}
        keyName={keyName}
      />
    )
  }
  if (typeof value === "object") {
    return (
      <ObjectNode
        entries={Object.entries(value as Record<string, unknown>)}
        depth={depth}
        collapseDeeperThan={collapseDeeperThan}
        trailingComma={trailingComma}
        keyName={keyName}
      />
    )
  }
  // Functions / symbols / undefined etc. — JSON-stringify-equivalent.
  return (
    <span>
      {keyPrefix}
      <LiteralToken text="undefined" />
      {trail}
    </span>
  )
}

function ObjectNode({
  entries,
  depth,
  collapseDeeperThan,
  trailingComma,
  keyName,
}: {
  entries: Array<[string, unknown]>
  depth: number
  collapseDeeperThan: number
  trailingComma: boolean
  keyName?: string
}) {
  if (entries.length === 0) {
    return (
      <span>
        {keyName !== undefined ? <KeyToken name={keyName} /> : null}
        <PunctuationToken text="{}" muted />
        {trailingComma ? <PunctuationToken text="," /> : null}
      </span>
    )
  }
  return (
    <DisclosureNode
      open={depth < collapseDeeperThan}
      summary={
        <>
          {keyName !== undefined ? <KeyToken name={keyName} /> : null}
          <PunctuationToken text="{" />
          <Summary count={entries.length} kind="keys" />
          <PunctuationToken text="}" />
          {trailingComma ? <PunctuationToken text="," /> : null}
        </>
      }
      open_prefix={
        <>
          {keyName !== undefined ? <KeyToken name={keyName} /> : null}
          <PunctuationToken text="{" />
        </>
      }
      open_suffix={
        <>
          <Indent depth={depth} />
          <PunctuationToken text="}" />
          {trailingComma ? <PunctuationToken text="," /> : null}
        </>
      }
    >
      {entries.map(([k, v], i) => (
        <div key={k}>
          <Indent depth={depth + 1} />
          <JsonNode
            value={v}
            depth={depth + 1}
            collapseDeeperThan={collapseDeeperThan}
            trailingComma={i < entries.length - 1}
            keyName={k}
          />
        </div>
      ))}
    </DisclosureNode>
  )
}

function ArrayNode({
  items,
  depth,
  collapseDeeperThan,
  trailingComma,
  keyName,
}: {
  items: unknown[]
  depth: number
  collapseDeeperThan: number
  trailingComma: boolean
  keyName?: string
}) {
  if (items.length === 0) {
    return (
      <span>
        {keyName !== undefined ? <KeyToken name={keyName} /> : null}
        <PunctuationToken text="[]" muted />
        {trailingComma ? <PunctuationToken text="," /> : null}
      </span>
    )
  }
  return (
    <DisclosureNode
      open={depth < collapseDeeperThan}
      summary={
        <>
          {keyName !== undefined ? <KeyToken name={keyName} /> : null}
          <PunctuationToken text="[" />
          <Summary count={items.length} kind="items" />
          <PunctuationToken text="]" />
          {trailingComma ? <PunctuationToken text="," /> : null}
        </>
      }
      open_prefix={
        <>
          {keyName !== undefined ? <KeyToken name={keyName} /> : null}
          <PunctuationToken text="[" />
        </>
      }
      open_suffix={
        <>
          <Indent depth={depth} />
          <PunctuationToken text="]" />
          {trailingComma ? <PunctuationToken text="," /> : null}
        </>
      }
    >
      {items.map((v, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: array indices are stable here
        <div key={i}>
          <Indent depth={depth + 1} />
          <JsonNode
            value={v}
            depth={depth + 1}
            collapseDeeperThan={collapseDeeperThan}
            trailingComma={i < items.length - 1}
          />
        </div>
      ))}
    </DisclosureNode>
  )
}

function DisclosureNode({
  open: defaultOpen,
  summary,
  open_prefix,
  open_suffix,
  children,
}: {
  open: boolean
  summary: React.ReactNode
  open_prefix: React.ReactNode
  open_suffix: React.ReactNode
  children: React.ReactNode
}) {
  const [open, setOpen] = useState(defaultOpen)
  return (
    <span>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="inline-flex items-baseline gap-1 text-left hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-(--accent)"
        data-testid="json-viewer-twisty"
      >
        <span aria-hidden="true" className="text-(--text2)">
          {open ? "▾" : "▸"}
        </span>
        {open ? open_prefix : summary}
      </button>
      {open ? (
        <>
          <div>{children}</div>
          <div>{open_suffix}</div>
        </>
      ) : null}
    </span>
  )
}

function Summary({ count, kind }: { count: number; kind: "keys" | "items" }) {
  return (
    <span className="px-1 text-(--text2) italic">
      ...{count} {kind}
    </span>
  )
}

function Indent({ depth }: { depth: number }) {
  return <span style={{ display: "inline-block", width: `${depth * 1.25}rem` }} />
}

function KeyToken({ name }: { name: string }) {
  return (
    <>
      <CodeToken kind="key" data-testid="json-key">
        "{name}"
      </CodeToken>
      <PunctuationToken text=": " />
    </>
  )
}

function StringToken({ value }: { value: string }) {
  return (
    <CodeToken kind="string" data-testid="json-string">
      "{escapeString(value)}"
    </CodeToken>
  )
}

function LiteralToken({ text }: { text: string }) {
  return (
    <CodeToken kind="literal" data-testid="json-literal">
      {text}
    </CodeToken>
  )
}

function PunctuationToken({ text, muted }: { text: string; muted?: boolean }) {
  return muted ? <CodeToken kind="punctuation">{text}</CodeToken> : <span>{text}</span>
}

function escapeString(s: string): string {
  // Show the same escapes JSON.stringify would emit for the most common
  // control characters. Avoids passing through literal newlines etc.
  return s
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\n/g, "\\n")
    .replace(/\r/g, "\\r")
    .replace(/\t/g, "\\t")
}
