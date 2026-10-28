import {
  ArrowRightIcon,
  BriefcaseIcon,
  DatabaseIcon,
  FolderIcon,
  HistoryIcon,
  LayersIcon,
  type LucideIcon,
  ZapIcon,
} from "lucide-react"

import { Kbd } from "@/components/data/Kbd"
import { cn } from "@/lib/utils"

import type { PaletteAction, PaletteItem, PaletteSections } from "./paletteItems"

const KIND_ICONS: Record<PaletteItem["kind"], LucideIcon> = {
  job: BriefcaseIcon,
  recent: HistoryIcon,
  connection: DatabaseIcon,
  queue: LayersIcon,
  group: FolderIcon,
  action: ZapIcon,
  settings: ArrowRightIcon,
}

/** The palette's list: the sections of results, or the actions ⇥ opened for one. */
export function PaletteResults({
  subject,
  subjectActions,
  grouped,
  flatItems,
  highlight,
  onHighlight,
  onSelect,
  onRunAction,
  emptyMessage,
}: {
  subject: PaletteItem | null
  subjectActions: PaletteAction[]
  grouped: PaletteSections
  flatItems: PaletteItem[]
  highlight: number
  onHighlight: (index: number) => void
  onSelect: (item: PaletteItem) => void
  onRunAction: (action: PaletteAction) => void
  emptyMessage: string
}) {
  return (
    <>
      {subject ? (
        <div className="px-2 pb-2" data-testid="command-palette-actions">
          <div className="px-2 pb-1 pt-2 text-2xs font-semibold tracking-wide text-(--text2) uppercase">
            Actions for {subject.label}
          </div>
          {subjectActions.map((action, idx) => (
            <button
              key={action.id}
              type="button"
              onClick={() => onRunAction(action)}
              onMouseEnter={() => onHighlight(idx)}
              data-active={idx === highlight ? "true" : undefined}
              className={cn(
                "flex w-full items-center gap-2.5 rounded-control px-2 py-1.5 text-left text-sm",
                idx === highlight ? "bg-(--surf2)" : "hover:bg-(--surf2)/60",
              )}
              data-testid={`command-palette-action-${action.id}`}
            >
              {action.label}
            </button>
          ))}
        </div>
      ) : null}
      {subject
        ? null
        : Object.entries(grouped).map(([group, items]) =>
            items.length === 0 ? null : (
              <div key={group} className="px-2 pb-2">
                <div className="px-2 pb-1 pt-2 text-2xs font-semibold uppercase tracking-wide text-(--text2)">
                  {group}
                </div>
                {items.map((item) => {
                  const idx = flatItems.indexOf(item)
                  const active = idx === highlight
                  const Icon = KIND_ICONS[item.kind]
                  return (
                    <button
                      key={`${group}-${item.id}`}
                      type="button"
                      onClick={() => onSelect(item)}
                      onMouseEnter={() => onHighlight(idx)}
                      data-active={active ? "true" : undefined}
                      className={cn(
                        "flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm",
                        active ? "bg-(--surf2) text-(--text)" : "hover:bg-(--surf)/50",
                      )}
                      data-testid={`command-palette-item-${item.id}`}
                    >
                      <Icon className="size-3.5 shrink-0 text-(--text2)" aria-hidden="true" />
                      <span className="min-w-0 break-words">{item.label}</span>
                      {item.hint ? (
                        <span className="ml-auto min-w-0 pl-4 text-right text-xs break-words text-(--text2)">
                          {item.hint}
                        </span>
                      ) : null}
                      {item.actions?.length && active ? (
                        <Kbd label="Tab for actions" className="shrink-0">
                          ⇥
                        </Kbd>
                      ) : null}
                    </button>
                  )
                })}
              </div>
            ),
          )}
      {!subject && flatItems.length === 0 ? (
        <p className="px-4 py-6 text-center text-sm text-(--text2)">{emptyMessage}</p>
      ) : null}
    </>
  )
}
