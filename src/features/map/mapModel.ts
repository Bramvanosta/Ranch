import { countOf, inLine } from "@/domain/stateCounts"
import type {
  JobStateInfo,
  Queue,
  QueueId,
  QueueWorker,
  Throughput,
  Topology,
} from "@/lib/tauri/bindings"

/**
 * The Map as data: queues on the left, worker pools on
 * the right, an edge from a queue to each pool that consumes it (weighted by
 * the queue's throughput, coloured by its failure rate, dashed when paused
 * or idle), a "No worker" node for queues nobody consumes, and flow links
 * between queues.
 */
export interface MapQueue {
  id: QueueId
  name: string
  paused: boolean
  waiting: number
  active: number
  /** Done + failed per minute over the last hour. */
  outPerMin: number
  /** Added per minute over the last hour; `capped` = a lower bound. */
  inPerMin: number | null
  inCapped: boolean
  /** failed / (done + failed) over the last hour, 0–1. */
  failureRate: number
  oldestWaitingMs: number | null
  workers: number
  active_: boolean
}

export interface MapPool {
  id: string
  name: string
  workers: number
  queues: string[]
  addrs: string[]
}

export interface MapEdge {
  queue: string
  /** A pool id, or `NO_WORKER`. */
  to: string
  /** Jobs per minute along the edge. */
  weight: number
  failureRate: number
  dashed: boolean
}

export interface MapFlow {
  parent: string
  child: string
  jobs: number
}

export interface MapModel {
  queues: MapQueue[]
  pools: MapPool[]
  noWorker: boolean
  edges: MapEdge[]
  flows: MapFlow[]
  /** Queues left out by "Only queues with activity". */
  hidden: number
}

export const NO_WORKER = "__no-worker__"

export function failureTone(rate: number): "ok" | "warn" | "danger" {
  if (rate >= 0.1) return "danger"
  if (rate >= 0.02) return "warn"
  return "ok"
}

/**
 * What a worker pool is called: BullMQ's worker name; else the queue it
 * serves (a BullMQ worker serves one queue). Grouping unnamed workers by host
 * put them all in one pool in containers, where every client comes from the
 * same gateway address ("Unnamed workers on 192.168.164.1").
 */
export function poolOf(w: QueueWorker, queue: string): { id: string; name: string } {
  if (w.name) return { id: `name:${w.name}`, name: w.name }
  return { id: `queue:${queue}`, name: `${queue} workers` }
}

export function buildMap({
  queues,
  throughput,
  workers,
  topology,
  onlyActive,
  states,
}: {
  queues: Queue[]
  throughput: Throughput | undefined
  workers: Map<string, QueueWorker[]>
  topology: Topology | undefined
  onlyActive: boolean
  /** The states the backend describes: which ones wait for a worker. */
  states: ReadonlyArray<JobStateInfo>
}): MapModel {
  const minutes = throughput
    ? Math.max(1, (throughput.bucketMs * throughput.bucketStartsMs.length) / 60_000)
    : 60
  const all: MapQueue[] = queues.map((q) => {
    const t = throughput?.queues.find((x) => x.queueId === q.id)
    const done = (t?.completed ?? []).reduce((a, b) => a + b, 0)
    const failed = (t?.failed ?? []).reduce((a, b) => a + b, 0)
    const stats = topology?.queues.find((x) => x.queueId === q.id)
    const waiting = inLine(q.counts, states)
    const active = countOf(q.counts, "active")
    const outPerMin = (done + failed) / minutes
    const inPerMin = stats ? stats.addedPerMinute : null
    return {
      id: q.id,
      name: q.name,
      paused: q.isPaused,
      waiting,
      active,
      outPerMin,
      inPerMin,
      inCapped: stats?.addedCapped ?? false,
      failureRate: done + failed > 0 ? failed / (done + failed) : 0,
      oldestWaitingMs: stats?.oldestWaitingMs ?? null,
      workers: (workers.get(q.name) ?? []).length,
      active_: outPerMin > 0 || waiting > 0 || active > 0 || (inPerMin ?? 0) > 0,
    }
  })
  const shown = (onlyActive ? all.filter((q) => q.active_) : all).sort(
    (a, b) => b.outPerMin - a.outPerMin || b.waiting - a.waiting || a.name.localeCompare(b.name),
  )
  const shownNames = new Set(shown.map((q) => q.name))

  const pools = new Map<string, MapPool>()
  const edges: MapEdge[] = []
  for (const q of shown) {
    const list = workers.get(q.name) ?? []
    const byPool = new Map<string, number>()
    for (const w of list) {
      const { id, name } = poolOf(w, q.name)
      const pool = pools.get(id) ?? { id, name, workers: 0, queues: [], addrs: [] }
      pool.workers += 1
      if (!pool.queues.includes(q.name)) pool.queues.push(q.name)
      if (!pool.addrs.includes(w.addr)) pool.addrs.push(w.addr)
      pools.set(id, pool)
      byPool.set(id, (byPool.get(id) ?? 0) + 1)
    }
    const dashed = q.paused || q.outPerMin === 0
    if (list.length === 0) {
      // Nobody consumes it: worth an edge only when it has something to do.
      if (q.waiting > 0 || q.outPerMin > 0) {
        edges.push({
          queue: q.name,
          to: NO_WORKER,
          weight: q.outPerMin,
          failureRate: q.failureRate,
          dashed: true,
        })
      }
      continue
    }
    for (const [id, count] of byPool) {
      edges.push({
        queue: q.name,
        to: id,
        weight: (q.outPerMin * count) / list.length,
        failureRate: q.failureRate,
        dashed,
      })
    }
  }

  // Pools in the order of the queues they consume (barycentre): fewer crossings.
  const indexOf = new Map(shown.map((q, i) => [q.name, i]))
  const orderedPools = [...pools.values()].sort((a, b) => {
    const at = (p: MapPool) =>
      p.queues.reduce((s, n) => s + (indexOf.get(n) ?? 0), 0) / p.queues.length
    return at(a) - at(b) || a.name.localeCompare(b.name)
  })

  const nameOf = new Map(queues.map((q) => [q.id, q.name]))
  const flows = (topology?.links ?? [])
    .map((l) => ({ parent: nameOf.get(l.parent), child: nameOf.get(l.child), jobs: l.jobs }))
    .filter(
      (l): l is { parent: string; child: string; jobs: number } =>
        l.parent !== undefined &&
        l.child !== undefined &&
        shownNames.has(l.parent) &&
        shownNames.has(l.child) &&
        l.parent !== l.child,
    )

  return {
    queues: shown,
    pools: orderedPools,
    noWorker: edges.some((e) => e.to === NO_WORKER),
    edges,
    flows,
    hidden: all.length - shown.length,
  }
}
