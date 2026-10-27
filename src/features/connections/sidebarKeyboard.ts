/**
 * Keyboard in the sidebar tree.
 *
 * Every row has one focus target marked `data-sidebar-row`, inside a
 * `data-sidebar-rowbox` that may hold a `data-sidebar-toggle` (the fold
 * button, with `aria-expanded`). Rows nest through `data-sidebar-item`.
 *
 * - ↑ / ↓ move to the previous / next row;
 * - → unfolds a folded row; ← folds an open row, or moves to its parent row;
 * - Enter opens (the browser's own behaviour on a link); ⌘Enter opens the
 *   row in place until native tabs exist.
 */
export function handleSidebarKeyDown(e: React.KeyboardEvent<HTMLElement>) {
  const target = e.target as HTMLElement
  const row = target.closest<HTMLElement>("[data-sidebar-row]")
  if (!row || !e.currentTarget.contains(row)) return
  // Alt+arrows reorder groups and environments: not ours.
  if (e.altKey) return

  const rows = [...e.currentTarget.querySelectorAll<HTMLElement>("[data-sidebar-row]")]
  const box = row.closest<HTMLElement>("[data-sidebar-rowbox]")
  const toggle = box?.querySelector<HTMLElement>("[data-sidebar-toggle]") ?? null
  const expanded = toggle?.getAttribute("aria-expanded")

  switch (e.key) {
    case "ArrowDown":
    case "ArrowUp": {
      const next = rows[rows.indexOf(row) + (e.key === "ArrowDown" ? 1 : -1)]
      if (next) {
        e.preventDefault()
        next.focus()
      }
      return
    }
    case "ArrowRight":
      if (toggle && expanded === "false") {
        e.preventDefault()
        toggle.click()
      }
      return
    case "ArrowLeft": {
      e.preventDefault()
      if (toggle && expanded === "true") {
        toggle.click()
        return
      }
      const item = row.closest<HTMLElement>("[data-sidebar-item]")
      const parent = item?.parentElement?.closest<HTMLElement>("[data-sidebar-item]")
      parent
        ?.querySelector<HTMLElement>("[data-sidebar-row]") // its own row comes first
        ?.focus()
      return
    }
    case "Enter":
      if (e.metaKey || e.ctrlKey) {
        // A modified click would open a browser tab; open in place instead.
        e.preventDefault()
        row.click()
      }
      return
  }
}
