/**
 * What this window's paint loop actually achieves.
 *
 * A file manager does not animate continuously, so "it feels like 60Hz" is
 * usually about scrolling and transitions rather than idle frames. This
 * measures the requestAnimationFrame interval while the window is visibly
 * animating, which is the only honest way to tell a capped webview from a
 * misperception. It reports the median interval, not the mean, so one slow
 * frame does not distort the answer.
 */
export interface FrameReport {
  frames: number;
  /** Median interval between frames, in milliseconds. */
  interval: number;
  /** Derived refresh rate. */
  hz: number;
  /** The slowest and fastest intervals, to show the spread. */
  slowest: number;
  fastest: number;
}

/**
 * Sample the paint loop for `durationMs` while nudging it with a moving
 * element, so the compositor has a reason to draw each frame.
 */
export function measureFrameRate(durationMs = 1200): Promise<FrameReport> {
  return new Promise((resolve) => {
    const intervals: number[] = [];
    let last = 0;
    const started = performance.now();

    const probe = document.createElement("div");
    probe.style.cssText =
      "position:fixed;left:0;top:0;width:1px;height:1px;opacity:0;pointer-events:none;will-change:transform";
    document.body.appendChild(probe);

    const tick = (now: number) => {
      if (last !== 0) intervals.push(now - last);
      last = now;
      // Force work every frame so the loop is not idle-throttled.
      probe.style.transform = `translate(${intervals.length % 2}px, 0)`;

      if (now - started < durationMs) {
        requestAnimationFrame(tick);
      } else {
        probe.remove();
        intervals.sort((a, b) => a - b);
        const median = intervals.length
          ? intervals[Math.floor(intervals.length / 2)]!
          : 0;
        resolve({
          frames: intervals.length,
          interval: median,
          hz: median > 0 ? Math.round(1000 / median) : 0,
          slowest: intervals[intervals.length - 1] ?? 0,
          fastest: intervals[0] ?? 0,
        });
      }
    };
    requestAnimationFrame(tick);
  });
}
