// The overflow detector: finds text that does not fit its box.
//
// `detectOverflow` runs inside the page through `page.evaluate`, so it must
// stay self-contained: no imports, no references to anything outside its body.

export type ViolationKind = "clip" | "spill" | "outside"

export interface Violation {
  /** `clip`: cut off or ellipsised; `spill`: runs into its neighbour; `outside`: leaves the window. */
  kind: ViolationKind
  /** How far the content overshoots, in CSS pixels. */
  px: number
  /** The first 80 characters of the element's visible text, whitespace collapsed. */
  text: string
  /** A short readable path to the element, for humans (not for matching). */
  selector: string
  /** How many elements on the screen share this kind and text. */
  count: number
}

export function detectOverflow(): Violation[] {
  const width = window.innerWidth
  const height = window.innerHeight
  const found: { el: HTMLElement; v: Violation }[] = []
  const outside = new Set<Element>()

  const describe = (el: HTMLElement): string => {
    const parts: string[] = []
    let node: HTMLElement | null = el
    while (node && node !== document.body && parts.length < 3) {
      const raw = node.getAttribute("data-testid") ?? node.getAttribute("aria-label")
      const label = raw?.replace(/\s+/g, " ").trim().slice(0, 32)
      const cls = typeof node.className === "string" ? node.className.split(/\s+/)[0] : ""
      let part = node.tagName.toLowerCase()
      if (label) part += `[${label}]`
      else if (cls) part += `.${cls}`
      parts.unshift(part)
      node = node.parentElement
    }
    return parts.join(" > ")
  }

  // An ancestor that scrolls or clips on this axis keeps the element inside it.
  const containedOn = (el: HTMLElement, axis: "x" | "y"): boolean => {
    for (let p = el.parentElement; p && p !== document.documentElement; p = p.parentElement) {
      const s = getComputedStyle(p)
      if ((axis === "x" ? s.overflowX : s.overflowY) !== "visible") return true
    }
    return false
  }

  for (const node of document.body.querySelectorAll("*")) {
    // SVG elements are not HTMLElements; text inside an <svg> is drawn, not laid out.
    if (!(node instanceof HTMLElement) || node.closest("svg")) continue
    const el = node
    if (!el.checkVisibility({ visibilityProperty: true, opacityProperty: true })) continue
    const s = getComputedStyle(el)
    // Inline boxes report clientWidth 0; their block ancestor is the one measured.
    if (s.display === "inline" || s.display === "contents") continue
    // Visually hidden text (Tailwind `sr-only` is a 1px × 1px clipped box).
    if (el.clientWidth <= 1 || el.clientHeight <= 1) continue
    const text = (el.innerText || "").replace(/\s+/g, " ").trim()
    if (!text) continue
    const short = text.slice(0, 80)
    const overX = el.scrollWidth - el.clientWidth

    const clipsX = s.overflowX === "hidden" || s.overflowX === "clip"
    const clamped = s.webkitLineClamp !== "none" && s.webkitLineClamp !== ""
    if ((clipsX || s.textOverflow === "ellipsis") && overX > 1) {
      found.push({
        el,
        v: { kind: "clip", px: overX, text: short, selector: describe(el), count: 1 },
      })
    } else if (clamped && el.scrollHeight - el.clientHeight > 1) {
      const px = el.scrollHeight - el.clientHeight
      found.push({ el, v: { kind: "clip", px, text: short, selector: describe(el), count: 1 } })
    } else if (
      (s.whiteSpace === "nowrap" || s.whiteSpace === "pre") &&
      s.overflowX === "visible" &&
      overX > 1
    ) {
      found.push({
        el,
        v: { kind: "spill", px: overX, text: short, selector: describe(el), count: 1 },
      })
    }

    // Report only the outermost element that leaves the window.
    if (el.parentElement && outside.has(el.parentElement)) {
      outside.add(el)
      continue
    }
    const r = el.getBoundingClientRect()
    const overRight = r.right - width
    const overBottom = r.bottom - height
    const leavesX = overRight > 1 && !containedOn(el, "x")
    const leavesY = overBottom > 1 && !containedOn(el, "y")
    if (leavesX || leavesY) {
      outside.add(el)
      const px = Math.round(Math.max(leavesX ? overRight : 0, leavesY ? overBottom : 0))
      found.push({ el, v: { kind: "outside", px, text: short, selector: describe(el), count: 1 } })
    }
  }

  // White-space is inherited, so a spilling cell also makes its nowrap ancestors
  // overflow: keep the innermost element only.
  const spills = found.filter((f) => f.v.kind === "spill").map((f) => f.el)
  const kept = found.filter(
    (f) => f.v.kind !== "spill" || !spills.some((other) => other !== f.el && f.el.contains(other)),
  )

  // Fifty identical truncated rows are one problem.
  const byKey = new Map<string, Violation>()
  for (const { v } of kept) {
    const key = `${v.kind}\u0000${v.text}`
    const seen = byKey.get(key)
    if (seen) {
      seen.count += 1
      seen.px = Math.max(seen.px, v.px)
    } else byKey.set(key, v)
  }
  return [...byKey.values()]
}
