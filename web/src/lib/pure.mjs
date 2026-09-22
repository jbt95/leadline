// Pure helpers shared by the dashboard and its tests. No React, no DOM, no
// imports: Node loads this directly for `npm test`, and Vite bundles it for
// the page. Everything here formats, sorts, filters, or derives chart geometry
// from canonical numbers. It never computes a metric, a score, or a ranking.

/**
 * Cumulative-bucket quantile, the same interpolation Prometheus'
 * `histogram_quantile` performs: rank q*count, then linear within the bucket
 * that first reaches it. The lower edge of the first bucket is zero, which
 * holds for durations, CPU, and memory.
 *
 * @param {number[]} bounds upper edge of each bucket
 * @param {number[]} cumulative count at or below each bucket's upper edge
 * @param {number} count total observations
 * @param {number} q fraction in 0..1
 * @returns {number | null} null when there is nothing to interpolate
 */
export function quantile(bounds, cumulative, count, q) {
  if (count <= 0 || bounds.length === 0 || cumulative.length !== bounds.length) return null;
  const rank = q * count;
  let idx = cumulative.findIndex((c) => c >= rank);
  if (idx === -1) idx = bounds.length - 1;
  const upper = bounds[idx];
  if (!Number.isFinite(upper)) return idx > 0 ? bounds[idx - 1] : null;
  const lower = idx > 0 ? bounds[idx - 1] : 0;
  const prev = idx > 0 ? cumulative[idx - 1] : 0;
  const width = cumulative[idx] - prev;
  if (width <= 0) return upper;
  return lower + ((rank - prev) / width) * (upper - lower);
}

/**
 * @typedef {{ count: number, sum: number, buckets: number[], bounds: number[] }} MergedHist
 */

/**
 * Merges rows of one histogram family that share identical bounds. Rows whose
 * bucket and bound lengths disagree are dropped; families that disagree on the
 * bounds themselves do not merge.
 *
 * @param {Array<{ bounds: number[], buckets: number[], count: number, sum: number }>} rows
 * @returns {MergedHist | null}
 */
export function mergeHistogram(rows) {
  const usable = rows.filter((r) => r.bounds.length > 0 && r.buckets.length === r.bounds.length);
  if (usable.length === 0) return null;
  const bounds = usable[0].bounds;
  if (!usable.every((r) => r.bounds.length === bounds.length && r.bounds.every((b, i) => b === bounds[i]))) {
    return null;
  }
  const buckets = bounds.map((_, i) => usable.reduce((t, r) => t + (r.buckets[i] ?? 0), 0));
  return {
    count: usable.reduce((t, r) => t + r.count, 0),
    sum: usable.reduce((t, r) => t + r.sum, 0),
    buckets,
    bounds,
  };
}

/**
 * Reads the route slug out of a `#/slug` hash. Anything else — a bare `#slug`
 * anchor, an unknown slug, an empty hash — falls back.
 *
 * @param {string} hash
 * @param {string[]} slugs every known route slug
 * @param {string} fallback the route to land on
 * @returns {string}
 */
export function slugFromHash(hash, slugs, fallback) {
  const raw = hash.startsWith("#/") ? hash.slice(2) : hash;
  const slug = raw.split("/")[0].split("?")[0];
  return slugs.includes(slug) ? slug : fallback;
}
