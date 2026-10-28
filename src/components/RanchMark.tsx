/**
 * The Ranch mark: three bars, the head of the queue (accent) sliding out of
 * the line. At 16 px and below it switches to a drawing on the pixel grid so
 * the bars stay crisp. Decorative: the name always sits next to it.
 */
export function RanchMark({ size = 20, className }: { size?: number; className?: string }) {
  if (size <= 16) {
    return (
      <svg
        width={size}
        height={size}
        viewBox="0 0 16 16"
        aria-hidden="true"
        className={className}
        data-variant="pixel"
      >
        <rect x="5" y="2" width="10" height="3" rx="1.5" fill="var(--accent)" />
        <rect x="1" y="7" width="10" height="3" rx="1.5" fill="currentColor" />
        <rect x="1" y="12" width="10" height="3" rx="1.5" fill="currentColor" />
      </svg>
    )
  }
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" className={className}>
      <rect x="11" y="3.5" width="18" height="6.5" rx="3.25" fill="var(--accent)" />
      <rect x="3" y="12.75" width="18" height="6.5" rx="3.25" fill="currentColor" />
      <rect x="3" y="22" width="18" height="6.5" rx="3.25" fill="currentColor" />
    </svg>
  )
}
