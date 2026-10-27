import { render, screen } from "@testing-library/react"
import { describe, expect, it } from "vitest"

import {
  DATA_TABLE_GRID,
  DataTable,
  type DataTableColumn,
  useColumnVisible,
  visibleColumns,
} from "./DataTable"

const columns: DataTableColumn[] = [
  { id: "name", track: "minmax(12rem, 2fr)" },
  { id: "added", track: "3.5rem", minWidth: 512 },
  { id: "duration", track: "5rem", minWidth: 672 },
]

function Row({ name }: { name: string }) {
  const showAdded = useColumnVisible("added")
  return (
    <div className={DATA_TABLE_GRID}>
      <span>{name}</span>
      {showAdded ? <span>2m</span> : null}
    </div>
  )
}

describe("DataTable", () => {
  it("drops the columns that do not fit, instead of squeezing them", () => {
    expect(visibleColumns(columns, 1100).map((c) => c.id)).toEqual(["name", "added", "duration"])
    expect(visibleColumns(columns, 600).map((c) => c.id)).toEqual(["name", "added"])
    expect(visibleColumns(columns, 400).map((c) => c.id)).toEqual(["name"])
  })

  it("one grid template for the header and every row", () => {
    render(
      <DataTable
        rows={["send-email", "send-digest"]}
        columns={columns}
        rowKey={(r) => r}
        rowHeight={40}
        ariaLabel="Jobs"
        header={<div className={DATA_TABLE_GRID} />}
        renderRow={(name) => <Row name={name} />}
      />,
    )
    const table = screen.getByRole("table", { name: "Jobs" })
    expect(table.style.getPropertyValue("--dt-cols")).toBe("minmax(12rem, 2fr) 3.5rem 5rem")
  })

  it("a row on its own shows every column", () => {
    render(<Row name="send-email" />)
    expect(screen.getByText("2m")).toBeInTheDocument()
  })
})
