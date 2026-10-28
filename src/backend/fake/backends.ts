import { BULLMQ_REDIS } from "@/domain/bullmqEndpoint"
import type { BackendInfo, CapabilityDescriptor } from "@/lib/tauri/bindings"

/**
 * What the BullMQ backend declares, as the Rust one lists it: everything
 * Ranch knows, in every state BullMQ keeps. The demo world and the tests
 * answer `list_backends` with it; screens read what `list_backends` answers,
 * through `src/capabilities/`, never this.
 */
export const BULLMQ_CAPABILITIES: CapabilityDescriptor = {
  jobStates: [
    { code: "waiting", label: "Waiting", tone: "pending", common: true },
    { code: "active", label: "Active", tone: "running", common: true },
    { code: "completed", label: "Completed", tone: "success", common: true },
    { code: "failed", label: "Failed", tone: "danger", common: true },
    { code: "delayed", label: "Delayed", tone: "warning", common: true },
    { code: "prioritized", label: "Prioritized", tone: "pending", common: false },
    { code: "waitingChildren", label: "Waiting children", tone: "warning", common: false },
    { code: "stalled", label: "Stalled", tone: "danger", common: false },
    { code: "paused", label: "Paused", tone: "warning", common: true },
  ],
  jobActions: { single: true, bulk: true, add: true },
  maintenance: { drain: true, clean: true, retryAllFailed: true },
  queueControl: true,
  schedules: { list: true, write: true, pause: true },
  workers: { list: true, count: true },
  flows: true,
  metrics: { queue: true, throughput: true, timeline: true, native: true },
  events: { read: true, follow: true },
  server: { info: true, topology: true },
}

/** The BullMQ kind as `list_backends` lists it: its form and what it can do. */
export const BULLMQ_REDIS_INFO: BackendInfo = {
  kind: BULLMQ_REDIS,
  label: "BullMQ on Redis",
  endpointSchema: {
    fields: [
      { key: "host", label: "Host", kind: "text", required: true, default: null },
      { key: "port", label: "Port", kind: "number", required: true, default: 6379 },
      { key: "db", label: "DB index", kind: "number", required: true, default: 0 },
      { key: "username", label: "Username", kind: "text", required: false, default: null },
      { key: "password", label: "Password", kind: "secret", required: false, default: null },
      { key: "tls", label: "Use TLS", kind: "boolean", required: false, default: false },
      {
        key: "tlsCustomCa",
        label: "Custom CA certificate",
        kind: "pem",
        required: false,
        default: null,
      },
      { key: "sshTunnel", label: "SSH tunnel", kind: "sshTunnel", required: false, default: null },
    ],
  },
  capabilities: BULLMQ_CAPABILITIES,
}
