import { writeFileSync } from "node:fs"
import { expect, test } from "@playwright/test"
import { loadAllowlist, loadResults, onThisPlatform, REPORT_FILE } from "./allowlist"
import { ROUTES, SIZES, THEMES } from "./routes"

test("the report is written and no allow-list entry is stale", () => {
  const entries = loadAllowlist()
  const results = loadResults()
  writeFileSync(REPORT_FILE, JSON.stringify(results, null, 2))

  // A filtered run (`--grep`) cannot tell a fixed failure from a screen it skipped.
  const expected = ROUTES.length * THEMES.length * SIZES.length
  test.skip(results.length < expected, `partial sweep (${results.length}/${expected} screens)`)

  const used = new Set(results.flatMap((r) => r.allowed))
  const stale = entries
    .map((e, i) => ({ e, i }))
    .filter(({ e, i }) => onThisPlatform(e) && !used.has(i))
    .map(
      ({ e }) =>
        `stale allow-list entry: ${e.route} ${e.kind} "${e.textStartsWith}" (${e.fixedBy})`,
    )
  expect(stale, "delete the entries your change fixed from e2e/overflow-allowlist.json").toEqual([])
})
