// The contrast detector: text that does not reach WCAG
// AA against what is behind it: 4.5:1, or 3:1 for large text (24 px, or
// 18.66 px bold). Colours are resolved by painting them on a canvas, so
// oklch, color-mix and alpha all come out as sRGB.
//
// `detectContrast` runs inside the page through `page.evaluate`: it must stay
// self-contained (no imports, nothing from outside its body).

export interface ContrastViolation {
  text: string
  ratio: number
  needed: number
  color: string
  background: string
  selector: string
  count: number
}

export function detectContrast(): ContrastViolation[] {
  const canvas = document.createElement("canvas")
  canvas.width = 1
  canvas.height = 1
  const ctx = canvas.getContext("2d", { willReadFrequently: true }) as CanvasRenderingContext2D
  const cache = new Map<string, [number, number, number, number]>()
  const rgba = (css: string): [number, number, number, number] => {
    const hit = cache.get(css)
    if (hit) return hit
    ctx.clearRect(0, 0, 1, 1)
    ctx.fillStyle = "#000"
    ctx.fillStyle = css
    ctx.fillRect(0, 0, 1, 1)
    const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data
    const value: [number, number, number, number] = [r, g, b, a / 255]
    cache.set(css, value)
    return value
  }
  const over = (top: [number, number, number, number], under: [number, number, number]) =>
    [0, 1, 2].map((i) => top[i] * top[3] + under[i] * (1 - top[3])) as [number, number, number]
  const lum = ([r, g, b]: [number, number, number]) => {
    const c = (v: number) => {
      const s = v / 255
      return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4
    }
    return 0.2126 * c(r) + 0.7152 * c(g) + 0.0722 * c(b)
  }
  const ratio = (a: [number, number, number], b: [number, number, number]) => {
    const [l1, l2] = [lum(a), lum(b)].sort((x, y) => y - x)
    return (l1 + 0.05) / (l2 + 0.05)
  }
  const rootBg = rgba(
    getComputedStyle(document.body).backgroundColor ||
      getComputedStyle(document.documentElement).getPropertyValue("--bg"),
  )
  const dim = rgba(
    getComputedStyle(document.documentElement).getPropertyValue("--dim").trim() || "transparent",
  )
  const background = (el: Element): [number, number, number] => {
    const layers: [number, number, number, number][] = []
    for (let node: Element | null = el; node; node = node.parentElement) {
      const c = rgba(getComputedStyle(node).backgroundColor)
      if (c[3] > 0) layers.push(c)
      if (c[3] >= 1) break
    }
    let base: [number, number, number] = [rootBg[0], rootBg[1], rootBg[2]]
    for (const layer of layers.reverse()) base = over(layer, base)
    return base
  }
  const hidden = (el: Element) => {
    for (let node: Element | null = el; node; node = node.parentElement) {
      const s = getComputedStyle(node)
      if (s.visibility === "hidden" || s.display === "none" || Number(s.opacity) === 0) return true
    }
    return false
  }
  const inactive = (el: Element) =>
    Boolean(el.closest("[disabled], [aria-disabled='true'], [data-disabled], [inert]"))
  const describe = (el: Element) => {
    const parts: string[] = []
    let node: Element | null = el
    while (node && node !== document.body && parts.length < 3) {
      const label = node.getAttribute("data-testid") ?? node.getAttribute("aria-label")
      parts.unshift(node.tagName.toLowerCase() + (label ? `[${label.slice(0, 32)}]` : ""))
      node = node.parentElement
    }
    return parts.join(" > ")
  }

  const found = new Map<string, ContrastViolation>()
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = (n.textContent ?? "").replace(/\s+/g, " ").trim()
    const el = n.parentElement
    if (!text || !el || el.closest("svg, [aria-hidden='true'], .sr-only")) continue
    const rect = el.getBoundingClientRect()
    if (rect.width === 0 || rect.height === 0 || hidden(el) || inactive(el)) continue
    const style = getComputedStyle(el)
    const fg = rgba(style.color)
    // `--dim` is for zero values only: not text to read.
    if (fg[0] === dim[0] && fg[1] === dim[1] && fg[2] === dim[2] && dim[3] > 0) continue
    const bg = background(el)
    const opacity = Number(style.opacity) || 1
    const color = over([fg[0], fg[1], fg[2], fg[3] * opacity], bg)
    const size = Number.parseFloat(style.fontSize)
    const bold = Number(style.fontWeight) >= 700
    const needed = size >= 24 || (bold && size >= 18.66) ? 3 : 4.5
    const r = ratio(color, bg)
    if (r + 0.005 >= needed) continue
    const key = `${text.slice(0, 60)}|${Math.round(r * 100)}`
    const seen = found.get(key)
    if (seen) {
      seen.count += 1
      continue
    }
    found.set(key, {
      text: text.slice(0, 80),
      ratio: Math.round(r * 100) / 100,
      needed,
      color: `rgb(${color.map(Math.round).join(", ")})`,
      background: `rgb(${bg.map(Math.round).join(", ")})`,
      selector: describe(el),
      count: 1,
    })
  }
  return [...found.values()]
}
