import { useVirtualizer } from "@tanstack/react-virtual"
import {
  type CSSProperties,
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react"

import { cn } from "@/lib/utils"

/**
 * One column of a `DataTable`: a CSS-grid track, and
 * the narrowest table width at which it still shows. Columns that do not fit
 * are left out of the template — they are not squeezed until they clip.
 */
export interface DataTableColumn {
  id: string
  /** A grid track: `"3.5rem"`, `"minmax(12rem, 2fr)"`… */
  track: string
  /** Hide the column when the table is narrower than this (px). */
  minWidth?: number
  /** The header cell's content; the table draws the header row from it. */
  header?: ReactNode
  /** Right-aligned (numbers, times). */
  align?: "end"
  /** How the table is sorted on this column, for assistive technology. */
  ariaSort?: "ascending" | "descending" | "none"
}

/** The columns that fit a table this wide, in order. */
export function visibleColumns(columns: readonly DataTableColumn[], width: number) {
  return columns.filter((c) => c.minWidth === undefined || width >= c.minWidth)
}

const VisibleColumns = createContext<ReadonlySet<string> | null>(null)

/**
 * Does column `id` show in the enclosing table? Outside a `DataTable` (a row
 * rendered on its own, in a test) every column shows.
 */
export function useColumnVisible(id: string): boolean {
  const visible = useContext(VisibleColumns)
  return visible === null || visible.has(id)
}

/** The ids of the columns that fit, or `null` outside a `DataTable` (all of them). */
export function useVisibleColumnIds(): ReadonlySet<string> | null {
  return useContext(VisibleColumns)
}

/** The classes a header or a row puts on itself to follow the table's columns. */
export const DATA_TABLE_GRID = "grid grid-cols-(--dt-cols) items-center gap-x-4"

/**
 * A virtualized table on a CSS grid: one column
 * template shared by the sticky header and every row, columns that fit the
 * width, a fixed row height, and the focused row kept in view.
 *
 * What a row looks like and does (selection, clicks, menus) is the caller's:
 * `renderRow` returns the row, which uses `DATA_TABLE_GRID` and
 * `useColumnVisible` for its cells. Sorting and selection state live with
 * the data's owner too; the table only lays them out.
 */
export function DataTable<T>({
  rows,
  columns,
  rowKey,
  rowHeight,
  header,
  renderRow,
  focusedIndex,
  ariaLabel,
  testId,
  className,
}: {
  rows: readonly T[]
  columns: readonly DataTableColumn[]
  rowKey: (row: T, index: number) => string
  /** The height of a one-line row; taller rows (wrapped text) are measured. */
  rowHeight: number
  /** A header row of the caller's; without it the table draws one from `columns[].header`. */
  header?: ReactNode
  renderRow: (row: T, index: number) => ReactNode
  /** The keyboard's row: scrolled into view when it changes. */
  focusedIndex?: number | null
  ariaLabel: string
  testId?: string
  className?: string
}) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const width = useElementWidth(scrollRef)
  const shown = useMemo(() => visibleColumns(columns, width), [columns, width])
  const visible = useMemo(() => new Set(shown.map((c) => c.id)), [shown])

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 5,
  })

  useEffect(() => {
    if (focusedIndex !== null && focusedIndex !== undefined && focusedIndex < rows.length) {
      virtualizer.scrollToIndex(focusedIndex, { align: "auto" })
    }
  }, [focusedIndex, rows.length, virtualizer])

  return (
    <VisibleColumns.Provider value={visible}>
      {/* biome-ignore lint/a11y/useSemanticElements: virtualized rows are absolutely positioned; a real <table> does not compose with that. */}
      <div
        ref={scrollRef}
        role="table"
        aria-label={ariaLabel}
        data-testid={testId}
        className={cn("min-h-0 flex-1 overflow-auto rounded-md border border-(--line)", className)}
        style={{ "--dt-cols": shown.map((c) => c.track).join(" ") } as CSSProperties}
      >
        {header ?? <DataTableHeader columns={shown} />}
        <div style={{ height: virtualizer.getTotalSize(), width: "100%", position: "relative" }}>
          {virtualizer.getVirtualItems().map((v) => {
            const row = rows[v.index]
            if (row === undefined) return null
            return (
              <div
                key={rowKey(row, v.index)}
                data-index={v.index}
                // Rows grow with wrapped text instead of cutting it.
                ref={virtualizer.measureElement}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  right: 0,
                  transform: `translateY(${v.start}px)`,
                }}
              >
                {renderRow(row, v.index)}
              </div>
            )
          })}
        </div>
      </div>
    </VisibleColumns.Provider>
  )
}

/** The element's width, followed as it resizes; very wide until first measured. */
function useElementWidth(ref: React.RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(Number.POSITIVE_INFINITY)
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    if (el.clientWidth > 0) setWidth(el.clientWidth)
    const observer = new ResizeObserver(([entry]) => {
      const w = entry?.contentRect.width
      if (w) setWidth(w)
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [ref])
  return width
}

/** The header row drawn from the columns: sticky, quiet, aligned like the cells. */
function DataTableHeader({ columns }: { columns: readonly DataTableColumn[] }) {
  return (
    // biome-ignore lint/a11y/useFocusableInteractive: the header row is presentation; its sort buttons are focusable.
    // biome-ignore lint/a11y/useSemanticElements: ARIA table of a virtualized list, see `DataTable`.
    <div
      role="row"
      data-testid="data-table-header"
      className={cn(
        DATA_TABLE_GRID,
        "sticky top-0 z-10 min-h-9 border-b border-(--line) bg-(--surf) px-3 text-2xs font-semibold tracking-wide text-(--text2) uppercase",
      )}
    >
      {columns.map((c) => (
        // biome-ignore lint/a11y/useFocusableInteractive: see the row.
        // biome-ignore lint/a11y/useSemanticElements: see the row.
        <span
          key={c.id}
          role="columnheader"
          aria-sort={c.ariaSort}
          className={cn("flex min-w-0 items-center", c.align === "end" && "justify-end text-right")}
        >
          {c.header}
        </span>
      ))}
    </div>
  )
}
