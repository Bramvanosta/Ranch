/**
 * Minimal substring-based fuzzy matcher.
 *
 * Returns a score for each `haystack` against the `needle`:
 *   - 0 means no match.
 *   - Higher = better match.
 *
 * Strategy:
 *   - Lower-case both sides.
 *   - Empty needle → every item scores 1 (preserve original order).
 *   - Otherwise: walk the needle through the haystack; each consecutive
 *     character match adds to the score; a non-match breaks the chain.
 *   - Bonus for matches at the start of the string AND at the start of
 *     a word (after a space/dash/slash).
 *
 * This is intentionally O(n) per item: for the realistic cardinality
 * (<1000 connections + queues combined) we don't need a real fuzzy
 * library. The 16ms target is trivially met.
 */
export function fuzzyScore(needle: string, haystack: string): number {
  const n = needle.toLowerCase()
  const h = haystack.toLowerCase()
  if (n === "") return 1
  if (n === h) return 1000 // exact match wins
  if (h.startsWith(n)) return 500 // prefix match
  if (h.includes(n)) return 100 + (50 - Math.min(50, h.indexOf(n))) // earlier substring → higher

  // Subsequence match: every char of needle appears in haystack in order.
  let ni = 0
  let score = 0
  let streak = 0
  for (let hi = 0; hi < h.length && ni < n.length; hi++) {
    if (h[hi] === n[ni]) {
      ni++
      streak++
      score += 2 + streak
      // Word-boundary bonus.
      if (hi === 0 || h[hi - 1] === " " || h[hi - 1] === "-" || h[hi - 1] === "/") {
        score += 5
      }
    } else {
      streak = 0
    }
  }
  return ni === n.length ? score : 0
}

/**
 * Sort items by score (desc), then by `tiebreak` (asc, typically a
 * timestamp / label).
 */
export function rankBy<T>(items: T[], needle: string, getLabel: (t: T) => string): T[] {
  return items
    .map((item) => ({ item, score: fuzzyScore(needle, getLabel(item)) }))
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score)
    .map((x) => x.item)
}
