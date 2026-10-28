import { describe, expect, it } from "vitest"

import { DEFAULT_SHARE_PATTERNS } from "@/domain/aiShare"

import type { Setting } from "./defineSetting"
import {
  aiEnabled,
  aiShareMode,
  aiSharePatterns,
  askAiCodeFolder,
  askAiDefaultAgent,
  attentionMutes,
  hintDismissed,
  jobPageSize,
  lastSelectedConnection,
  mcpActivityEnabled,
  notificationThresholds,
  paletteRecents,
  refreshAuto,
  refreshIntervalMs,
  welcomeDismissed,
} from "./settings"

/** What the setting reads back after writing `value`. */
function roundTrip<T>(setting: Setting<T>, value: T): T {
  return setting.parse(setting.serialize(value))
}

describe("stored keys", () => {
  it("stay the keys existing installs already hold", () => {
    expect(
      [
        refreshAuto,
        refreshIntervalMs,
        jobPageSize,
        lastSelectedConnection,
        hintDismissed("cross-queue-scope"),
        welcomeDismissed,
        paletteRecents,
        attentionMutes,
        notificationThresholds,
        aiEnabled,
        aiShareMode,
        aiSharePatterns,
        mcpActivityEnabled,
        askAiDefaultAgent,
        askAiCodeFolder("g-1"),
      ].map((s) => s.key),
    ).toEqual([
      "refresh.auto",
      "refresh.intervalMs",
      "jobs.pageSize",
      "selected_connection_id",
      "hint:cross-queue-scope:dismissed",
      "wizard.dismissed",
      "palette.recents",
      "attention_mutes",
      "notifications.thresholds",
      "ai_enabled",
      "ai_share_mode",
      "ai_share_patterns",
      "mcp_activity_enabled",
      "ask_ai_default_agent",
      "ask_ai_folder:g-1",
    ])
  })
})

describe("refreshAuto", () => {
  it("is on unless stored as 0, and stores 1 / 0", () => {
    expect(refreshAuto.parse(null)).toBe(true)
    expect(refreshAuto.parse("0")).toBe(false)
    expect(refreshAuto.parse("1")).toBe(true)
    expect(refreshAuto.serialize(false)).toBe("0")
    expect(refreshAuto.serialize(true)).toBe("1")
  })
})

describe("refreshIntervalMs", () => {
  it("keeps an offered interval and falls back to a second otherwise", () => {
    expect(refreshIntervalMs.parse("10000")).toBe(10_000)
    expect(refreshIntervalMs.parse("1234")).toBe(1_000)
    expect(refreshIntervalMs.parse("fast")).toBe(1_000)
    expect(refreshIntervalMs.parse(null)).toBe(1_000)
    expect(refreshIntervalMs.serialize(5_000)).toBe("5000")
  })
})

describe("jobPageSize", () => {
  it("keeps an offered page size and falls back to 50", () => {
    expect(jobPageSize.parse("100")).toBe(100)
    expect(jobPageSize.parse("33")).toBe(50)
    expect(jobPageSize.parse(null)).toBe(50)
    expect(roundTrip(jobPageSize, 250)).toBe(250)
  })
})

describe("lastSelectedConnection", () => {
  it("stores no selection as an empty string, and reads back what is stored", () => {
    expect(lastSelectedConnection.serialize(null)).toBe("")
    expect(lastSelectedConnection.parse("")).toBe("")
    expect(lastSelectedConnection.parse(null)).toBeNull()
    expect(roundTrip(lastSelectedConnection, "conn-1")).toBe("conn-1")
  })
})

describe("hint and welcome flags", () => {
  it("are on for 1 only, and are cleared with 0", () => {
    for (const flag of [hintDismissed("cross-queue-scope"), welcomeDismissed]) {
      expect(flag.parse(null)).toBe(false)
      expect(flag.parse("1")).toBe(true)
      expect(flag.parse("0")).toBe(false)
      expect(flag.serialize(true)).toBe("1")
      expect(flag.serialize(false)).toBe("0")
    }
  })
})

describe("paletteRecents", () => {
  it("reads a JSON list and ignores anything else", () => {
    const recent = {
      kind: "connection" as const,
      id: "c1",
      label: "local",
      to: "/connections/c1",
      ts: "2026-05-11T12:00:00Z",
    }
    expect(roundTrip(paletteRecents, [recent])).toEqual([recent])
    expect(paletteRecents.serialize([])).toBe("[]")
    expect(paletteRecents.parse("{oops")).toEqual([])
    expect(paletteRecents.parse('{"a":1}')).toEqual([])
    expect(paletteRecents.parse(null)).toEqual([])
  })
})

describe("attentionMutes", () => {
  it("reads a JSON map of mutes and ignores anything else", () => {
    expect(roundTrip(attentionMutes, { "failing:c1:emails": 1_000 })).toEqual({
      "failing:c1:emails": 1_000,
    })
    expect(attentionMutes.parse("")).toEqual({})
    expect(attentionMutes.parse("[1]")).toEqual({})
    expect(attentionMutes.parse(null)).toEqual({})
  })
})

describe("notificationThresholds", () => {
  it("reads a JSON list and ignores anything else", () => {
    const threshold = { id: "t1", metric: "failed" as const, op: "gt" as const, value: 10 }
    expect(roundTrip(notificationThresholds, [threshold])).toEqual([threshold])
    expect(notificationThresholds.parse("not json")).toEqual([])
    expect(notificationThresholds.parse('{"id":"t1"}')).toEqual([])
    expect(notificationThresholds.parse(null)).toEqual([])
  })
})

describe("aiEnabled", () => {
  it("is on unless stored as false", () => {
    expect(aiEnabled.parse(null)).toBe(true)
    expect(aiEnabled.parse("true")).toBe(true)
    expect(aiEnabled.parse("false")).toBe(false)
  })
})

describe("aiShareMode and aiSharePatterns", () => {
  it("mask by default, with the default patterns", () => {
    expect(aiShareMode.parse(null)).toBe("mask")
    expect(aiShareMode.parse("names")).toBe("names")
    expect(aiShareMode.parse("everything")).toBe("everything")
    expect(aiShareMode.parse("all of it")).toBe("mask")
    expect(aiSharePatterns.parse(null)).toBe(DEFAULT_SHARE_PATTERNS)
    expect(aiSharePatterns.parse("")).toBe("")
    expect(roundTrip(aiSharePatterns, "email, token")).toBe("email, token")
  })
})

describe("mcpActivityEnabled", () => {
  it("records unless stored as false, and stores true / false", () => {
    expect(mcpActivityEnabled.parse(null)).toBe(true)
    expect(mcpActivityEnabled.parse("false")).toBe(false)
    expect(mcpActivityEnabled.serialize(false)).toBe("false")
    expect(mcpActivityEnabled.serialize(true)).toBe("true")
  })
})

describe("askAiDefaultAgent", () => {
  it("is Claude Code unless a known agent is stored", () => {
    expect(askAiDefaultAgent.parse(null)).toBe("claudeCode")
    expect(askAiDefaultAgent.parse("cursor")).toBe("cursor")
    expect(askAiDefaultAgent.parse("notepad")).toBe("claudeCode")
  })
})

describe("askAiCodeFolder", () => {
  it("is trimmed, and blank reads as no folder", () => {
    const folder = askAiCodeFolder("g-1")
    expect(folder.parse("  /code/app  ")).toBe("/code/app")
    expect(folder.parse("   ")).toBeNull()
    expect(folder.parse(null)).toBeNull()
    expect(folder.serialize("  /code/app ")).toBe("/code/app")
  })
})
