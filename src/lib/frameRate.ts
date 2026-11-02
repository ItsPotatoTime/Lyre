/**
 * What this window's paint loop actually achieves, while it is doing it.
 *
 * A file manager does not animate continuously, so "it feels like 60 Hz" is
 * usually about scrolling and transitions rather than idle frames. This samples
 * the requestAnimationFrame interval continuously while the readout is on,
 * which is the only honest way to tell a capped webview from a misperception:
 * the number moves when the window is under load, so a rate that sits still
 * through a fast scroll is a cap and not a measurement artefact.
 *
 * It reports the median interval, not the mean, so one slow frame does not
 * distort the answer.
 */
import { useEffect, useState } from "react";

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

/** How much of the past one readout covers. */
const WINDOW_MS = 1000;

/**
 * How often the meter is allowed to draw, at most.
 *
 * The probe has to be painted, or the sample is of a window the compositor is
 * sparing work for. It must not be painted on every frame: a display with
 * adaptive sync runs at the rate the window presents, so a meter that drew on
 * every vblank held the panel at, and a fraction past, its own maximum, and a
 * panel oscillating at the top of its variable range answers with visible
 * flicker over the whole screen. Thirty in a second is clear of any panel's
 * ceiling and still keeps the window drawing.
 */
const DAMAGE_MS = 1000 / 30;

/**
 * Follow the paint loop while `enabled`.
 *
 * Nothing runs when the readout is off, and nothing is remembered when it is
 * switched off: this is a diagnostic, not a setting, and a meter left running
 * would keep the compositor busy for as long as the window is open.
 */
export function useFrameRate(enabled: boolean): FrameReport | null {
  const [report, setReport] = useState<FrameReport | null>(null);

  useEffect(() => {
    setReport(null);
    if (!enabled) return;

    let frame = 0;
    let last = 0;
    let windowStart = 0;
    let lastDraw = 0;
    let drawn = 0;
    let intervals: number[] = [];

    // A pixel that actually gets painted, so the compositor has a reason to
    // draw and the sample is not of an idle page being spared work. It sits
    // inside the status bar, painted in the bar's own colour, so the paint is
    // real and the mark it leaves is not: only the change, never the pixel.
    const probe = document.createElement("div");
    probe.style.cssText =
      "position:fixed;left:0;bottom:0;width:1px;height:1px;background:var(--bg-status);pointer-events:none;will-change:transform";
    document.body.appendChild(probe);

    const tick = (now: number) => {
      if (last === 0) {
        last = now;
        windowStart = now;
        lastDraw = now;
      } else {
        intervals.push(now - last);
        last = now;
      }

      if (now - lastDraw >= DAMAGE_MS) {
        lastDraw = now;
        drawn += 1;
        probe.style.transform = `translateY(${drawn % 2}px)`;
      }

      if (now - windowStart >= WINDOW_MS && intervals.length > 0) {
        const sorted = [...intervals].sort((a, b) => a - b);
        const median = sorted[Math.floor(sorted.length / 2)]!;
        // Counted over the span the frames actually covered, rather than
        // divided out of a median interval. On a 300 Hz panel the interval is
        // 3.33 ms, and a frame clock quantised to a tenth of a millisecond
        // reports 3.3, which divides out to 303: a rate past the panel's
        // maximum on a panel doing exactly its own rate.
        const span = now - windowStart;
        setReport({
          frames: sorted.length,
          interval: median,
          hz: span > 0 ? Math.round((intervals.length * 1000) / span) : 0,
          slowest: sorted[sorted.length - 1]!,
          fastest: sorted[0]!,
        });
        intervals = [];
        windowStart = now;
      }
      frame = requestAnimationFrame(tick);
    };

    frame = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(frame);
      probe.remove();
    };
  }, [enabled]);

  return report;
}
