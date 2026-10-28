import { describe, expect, it } from "vitest"

/**
 * Screens read the job states a backend describes (`useJobStates`), or take
 * them as an argument: production code under `src/` names none of BullMQ's
 * own states. The demo world (`backend/fake/`), the tests, their helpers
 * and the generated bindings may. A file whose hit is not a state that
 * logic depends on goes in the allow-list, with the reason.
 */

const sources = import.meta.glob<string>(["/src/**/*.{ts,tsx}", "!/src/**/*.d.ts"], {
  query: "?raw",
  import: "default",
  eager: true,
})

const BULLMQ_ONLY = /\b(prioritized|waitingChildren|stalled)\b|waiting-children/g

const ALLOWED: Record<string, string> = {
  "src/features/jobs/progressWatch.ts":
    "`stalled` is a running job whose progress stopped moving, not BullMQ's stalled state",
  "src/features/jobs/ActiveJobCells.tsx": "the same progress stall, shown on the row",
  "src/features/jobs/JobDrillDown.tsx": "the same progress stall, shown on the job page",
  "src/features/jobs/buildJobTimeline.ts":
    "BullMQ's wording of a parent waiting for its children; the waiting states come from the description",
  "src/features/jobs/addJobForm.ts":
    "the add-job form's priority is BullMQ's job option, and the line says where BullMQ puts such a job",
  "src/features/flows/flowLayout.ts":
    "a flow's parents waiting for their children, in BullMQ's parent and child model",
  "src/features/flows/FlowGraph.tsx": "the same parents, counted in the flow's header line",
  "src/features/overview/LiveEvents.tsx":
    "`stalled` is an event name of BullMQ's event streams (its dot colour), not a job state",
}

const isProduction = (file: string) =>
  !/\.test\.tsx?$/.test(file) &&
  !file.startsWith("src/backend/fake/") &&
  !file.startsWith("src/test/") &&
  file !== "src/lib/tauri/bindings.ts"

/** The source without its comments: a comment may name a state. */
const code = (source: string) =>
  source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'`])\/\/.*$/gm, "$1")

const files = Object.entries(sources).map(([absolute, source]) => ({
  file: absolute.replace(/^\//, ""),
  source,
}))

describe("job states in production code", () => {
  it("name none of BullMQ's own states outside the allow-list", () => {
    const hits = files
      .filter(({ file }) => isProduction(file) && !(file in ALLOWED))
      .flatMap(({ file, source }) =>
        [...new Set([...code(source).matchAll(BULLMQ_ONLY)].map((m) => m[0]))].map(
          (state) => `${file} → ${state}`,
        ),
      )
    expect(hits).toEqual([])
  })

  it("keeps an allow-list of files that still name one", () => {
    const stale = Object.keys(ALLOWED).filter((file) => {
      const found = files.find((f) => f.file === file)
      return !found || [...code(found.source).matchAll(BULLMQ_ONLY)].length === 0
    })
    expect(stale).toEqual([])
  })
})
