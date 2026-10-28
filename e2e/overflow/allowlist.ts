// Known overflow failures, kept green until the change that fixes them lands
// (`e2e/overflow-allowlist.json`). The list only shrinks: an entry that no
// longer matches anything fails the run.

import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import type { Violation, ViolationKind } from "./detect"

export interface AllowEntry {
  /** A route name from `routes.ts`, or "*" for a failure of the shell seen on every screen. */
  route: string
  kind: ViolationKind
  /** Matches a violation whose text starts with this (whitespace collapsed). */
  textStartsWith: string
  reason: string
  /** What is expected to fix it (an issue or pull request). */
  fixedBy: string
  /** Only for one theme or one size, when the failure is specific. */
  theme?: string
  size?: string
  /**
   * Only on one OS (`process.platform`): system fonts differ, so a
   * label can fit on Linux (CI) and not on macOS.
   */
  platform?: NodeJS.Platform
}

/** Entries that apply on the machine running the check. */
export const onThisPlatform = (entry: AllowEntry) =>
  entry.platform === undefined || entry.platform === process.platform

export interface ScreenResult {
  route: string
  theme: string
  size: string
  violations: Violation[]
  /** Indexes of the allow-list entries this screen used. */
  allowed: number[]
}

const root = new URL("../", import.meta.url)
export const ALLOWLIST_FILE = new URL("overflow-allowlist.json", root)
export const REPORT_FILE = new URL(".overflow-report.json", root)
// One file per screen, so a retried test (in a fresh worker) replaces its own.
export const RESULTS_DIR = new URL("../test-results/overflow-sweep/", root)

export const loadAllowlist = (): AllowEntry[] =>
  JSON.parse(readFileSync(ALLOWLIST_FILE, "utf8")) as AllowEntry[]

const collapse = (s: string) => s.replace(/\s+/g, " ").trim()

export function matches(
  entry: AllowEntry,
  v: Violation,
  where: { route: string; theme: string; size: string },
): boolean {
  return (
    onThisPlatform(entry) &&
    (entry.route === "*" || entry.route === where.route) &&
    entry.kind === v.kind &&
    (entry.theme === undefined || entry.theme === where.theme) &&
    (entry.size === undefined || entry.size === where.size) &&
    collapse(v.text).startsWith(collapse(entry.textStartsWith))
  )
}

/** Splits a screen's violations into the ones the allow-list covers and the rest. */
export function applyAllowlist(
  entries: AllowEntry[],
  violations: Violation[],
  where: { route: string; theme: string; size: string },
): { blocking: Violation[]; allowed: number[] } {
  const allowed = new Set<number>()
  const blocking: Violation[] = []
  for (const v of violations) {
    const i = entries.findIndex((e) => matches(e, v, where))
    if (i === -1) blocking.push(v)
    else allowed.add(i)
  }
  return { blocking, allowed: [...allowed] }
}

export function saveResult(result: ScreenResult) {
  mkdirSync(RESULTS_DIR, { recursive: true })
  const file = `${result.route}--${result.theme}--${result.size}.json`
  writeFileSync(new URL(file, RESULTS_DIR), JSON.stringify(result, null, 2))
}

export function loadResults(): ScreenResult[] {
  let files: string[]
  try {
    files = readdirSync(RESULTS_DIR)
  } catch {
    return []
  }
  return files
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(readFileSync(new URL(f, RESULTS_DIR), "utf8")) as ScreenResult)
}

export const formatViolation = (
  v: Violation,
  where: { route: string; theme: string; size: string },
) =>
  `${where.route} [${where.theme} ${where.size}] ${v.kind} +${v.px}px` +
  `${v.count > 1 ? ` (×${v.count})` : ""} "${v.text}" in ${v.selector}`
