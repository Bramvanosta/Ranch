import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import { groupAttempts, StackTraceViewer } from "./StackTraceViewer"

const STACK = (message: string) =>
  [
    `Error: ${message}`,
    "    at sendEmail (/app/dist/workers/email.js:41:13)",
    "    at async Worker.callProcessJob (/app/node_modules/bullmq/dist/cjs/classes/worker.js:178:21)",
    "    at process.processTicksAndRejections (node:internal/process/task_queues:95:5)",
  ].join("\n")

describe("StackTraceViewer", () => {
  it("message in --state-failed, your frames plain, library frames dimmed", () => {
    render(<StackTraceViewer stacktrace={[STACK("connection reset")]} failedReason={null} />)
    const lines = screen.getByTestId("stacktrace-block").querySelectorAll("div")
    expect(lines).toHaveLength(4)
    expect(lines[0]).toHaveStyle({ color: "var(--state-failed)" })
    // The frame in the user's own code is what the eye should land on.
    expect(lines[1]?.className).not.toContain("text-(--text2)")
    expect(lines[1]).not.toHaveAttribute("data-library")
    // Dependencies and Node internals step back.
    expect(lines[2]).toHaveAttribute("data-library", "true")
    expect(lines[2]?.className).toContain("text-(--text2)")
    expect(lines[3]).toHaveAttribute("data-library", "true")
  })

  it("one block per failed attempt — the latest first and open, the earlier ones folded", () => {
    render(
      <StackTraceViewer
        stacktrace={[STACK("first"), STACK("second"), STACK("third")]}
        failedReason="third"
        showReason={false}
      />,
    )
    const attempts = screen.getAllByTestId("stacktrace-attempt")
    expect(attempts).toHaveLength(3)
    expect(attempts[0]).toHaveTextContent("Failure 3 of 3")
    expect(attempts[0]).toHaveTextContent("latest")
    expect(attempts[0]).toHaveAttribute("open")
    expect(attempts[1]).toHaveTextContent("Failure 2 of 3")
    expect(attempts[1]).not.toHaveAttribute("open")
    // Folded, an attempt still says what it failed with.
    expect(attempts[2]).toHaveTextContent("Error: first")
    // The page shows the reason above the tabs: not repeated here.
    expect(screen.queryByTestId("stacktrace-reason")).not.toBeInTheDocument()
  })

  it("reads a trace written line by line as one block", () => {
    expect(groupAttempts(["Error: x", "    at fn (a.js:1:1)"])).toEqual([
      ["Error: x", "    at fn (a.js:1:1)"],
    ])
    // …and one-line errors as one attempt each.
    expect(groupAttempts(["Error: a", "Error: b"])).toEqual([["Error: a"], ["Error: b"]])
    expect(groupAttempts([])).toEqual([])
  })

  it("says so when a failure left no stack trace", () => {
    render(<StackTraceViewer stacktrace={[]} failedReason="Reason only" showReason={false} />)
    expect(screen.getByTestId("stacktrace-none")).toHaveTextContent("No stack trace was recorded")
  })

  it("renders the failedReason banner when set", () => {
    render(<StackTraceViewer stacktrace={["Error: oops"]} failedReason="Connection refused" />)
    expect(screen.getByTestId("stacktrace-reason")).toHaveTextContent("Connection refused")
  })

  it("renders the failedReason banner even when stacktrace is empty", () => {
    render(<StackTraceViewer stacktrace={[]} failedReason="Reason only" />)
    expect(screen.getByTestId("stacktrace-reason")).toBeInTheDocument()
    expect(screen.queryByTestId("stacktrace-block")).not.toBeInTheDocument()
  })

  it("renders an empty-state when both stacktrace and failedReason are empty", () => {
    render(<StackTraceViewer stacktrace={[]} failedReason={null} />)
    expect(screen.getByTestId("stacktrace-empty")).toBeInTheDocument()
    expect(screen.queryByTestId("stacktrace-block")).not.toBeInTheDocument()
  })

  it("decorates file:line:col frames with a title hint for the deferred file-open", () => {
    render(
      <StackTraceViewer
        stacktrace={["Error: x", "    at fn (/usr/src/worker.js:42:11)"]}
        failedReason={null}
      />,
    )
    const frame = screen.getByTestId("stacktrace-frame")
    expect(frame).toHaveAttribute("title", "File-open from stack frames lands in v1.x")
    expect(frame.textContent).toBe("/usr/src/worker.js:42:11")
  })
})
