/** Query keys of a connection's topology: which queues feed which workers. */
export const topologyKeys = {
  of: (connectionId: string) => ["topology", connectionId] as const,
}
