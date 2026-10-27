import { getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window"
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification"

/**
 * Get the human's attention when an agent is blocked on them
 *. Every step is best-effort and independent: a platform that refuses
 * one (a Linux WM ignoring always-on-top, notifications denied…) must not
 * stop the others, and none of it may ever break the modal itself.
 */

async function attempt(step: () => Promise<unknown>): Promise<void> {
  try {
    await step()
  } catch {
    // Best-effort by design.
  }
}

export interface AttentionOptions {
  alwaysOnTop: boolean
  notifySound: boolean
  title: string
  body: string
}

/** Bring Ranch forward. Call when a NEW request arrives. */
export async function raiseAttention(options: AttentionOptions): Promise<void> {
  const win = getCurrentWindow()
  const focused = await win.isFocused().catch(() => false)

  if (options.alwaysOnTop) {
    await attempt(() => win.unminimize())
    await attempt(() => win.show())
    await attempt(() => win.setAlwaysOnTop(true))
    await attempt(() => win.setFocus())
  }
  if (!focused) {
    await attempt(() => win.requestUserAttention(UserAttentionType.Critical))
  }
  if (options.notifySound) {
    // The OS notification only when Ranch is not already in front; the sound
    // always — the user may be looking at another monitor.
    if (!focused) {
      await attempt(async () => {
        const granted = (await isPermissionGranted()) || (await requestPermission()) === "granted"
        if (granted) sendNotification({ title: options.title, body: options.body })
      })
    }
    beep()
  }
}

/** Undo the temporary always-on-top. Call when nothing is pending anymore. */
export async function releaseAttention(): Promise<void> {
  const win = getCurrentWindow()
  await attempt(() => win.setAlwaysOnTop(false))
  await attempt(() => win.requestUserAttention(null))
}

/** Two short tones via WebAudio — no asset to ship, silent if audio is blocked. */
function beep(): void {
  try {
    const Ctx =
      window.AudioContext ??
      (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
    if (!Ctx) return
    const ctx = new Ctx()
    const now = ctx.currentTime
    for (const [offset, frequency] of [
      [0, 880],
      [0.16, 1175],
    ] as const) {
      const osc = ctx.createOscillator()
      const gain = ctx.createGain()
      osc.frequency.value = frequency
      gain.gain.setValueAtTime(0.0001, now + offset)
      gain.gain.exponentialRampToValueAtTime(0.12, now + offset + 0.02)
      gain.gain.exponentialRampToValueAtTime(0.0001, now + offset + 0.14)
      osc.connect(gain).connect(ctx.destination)
      osc.start(now + offset)
      osc.stop(now + offset + 0.15)
    }
    window.setTimeout(() => void ctx.close(), 600)
  } catch {
    // Audio unavailable: the visual cues remain.
  }
}
