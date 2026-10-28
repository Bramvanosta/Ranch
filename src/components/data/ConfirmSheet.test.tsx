import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { ConfirmSheet } from "./ConfirmSheet"

function sheet(props: Partial<Parameters<typeof ConfirmSheet>[0]> = {}) {
  const onConfirm = vi.fn()
  const onOpenChange = vi.fn()
  render(
    <ConfirmSheet
      open
      onOpenChange={onOpenChange}
      title="Drain emails?"
      confirmLabel="Drain 400 jobs"
      onConfirm={onConfirm}
      confirmTestId="confirm"
      cancelTestId="cancel"
      {...props}
    />,
  )
  return { onConfirm, onOpenChange }
}

describe("ConfirmSheet", () => {
  it("says what goes and what stays, with counts", () => {
    sheet({
      danger: true,
      removed: [{ label: "Waiting jobs", count: 400 }],
      kept: [{ label: "Failed jobs", count: 1200 }],
    })
    const counts = screen.getByTestId("confirm-sheet-counts")
    expect(counts).toHaveTextContent("Removed")
    expect(counts).toHaveTextContent("Waiting jobs400")
    expect(counts).toHaveTextContent("Kept")
    expect(counts).toHaveTextContent((1200).toLocaleString())
    expect(screen.getByTestId("confirm")).toHaveTextContent("Drain 400 jobs")
  })

  it("focuses Cancel when the action destroys something", async () => {
    sheet({ danger: true })
    await waitFor(() => expect(screen.getByTestId("cancel")).toHaveFocus())
  })

  it("focuses the action when it can be undone", async () => {
    sheet()
    await waitFor(() => expect(screen.getByTestId("confirm")).toHaveFocus())
  })

  it("on a protected connection, the name must be typed before confirming", () => {
    const { onConfirm } = sheet({ danger: true, protectedName: "emails" })
    expect(screen.getByText(/This connection is protected/)).toBeInTheDocument()
    const confirm = screen.getByTestId("confirm")
    expect(confirm).toBeDisabled()

    fireEvent.change(screen.getByTestId("confirm-sheet-name"), { target: { value: "email" } })
    expect(confirm).toBeDisabled()
    fireEvent.change(screen.getByTestId("confirm-sheet-name"), { target: { value: "emails" } })
    expect(confirm).toBeEnabled()
    fireEvent.click(confirm)
    expect(onConfirm).toHaveBeenCalledTimes(1)
  })

  it("Cancel closes without confirming", () => {
    const { onConfirm, onOpenChange } = sheet({ danger: true })
    fireEvent.click(screen.getByTestId("cancel"))
    expect(onOpenChange).toHaveBeenCalledWith(false)
    expect(onConfirm).not.toHaveBeenCalled()
  })
})
