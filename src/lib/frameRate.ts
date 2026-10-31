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
    let intervals: number[] = [];

    // A frame that actually changes something, so the compositor has a reason
    // to draw and the sample is not of an idle page being spared work.
    const probe = document.createElement("div");
    probe.style.cssText =
      "position:fixed;left:0;top:0;width:1px;height:1px;opacity:0;pointer-events:none;will-change:transform";
    document.body.appendChild(probe);

    const tick = (now: number) => {
      if (last === 0) {
        last = now;
        windowStart = now;
      } else {
        intervals.push(now - last);
        last = now;
      }
      probe.style.transform = `translate(${intervals.length % 2}px, 0)`;

      if (now - windowStart >= WINDOW_MS && intervals.length > 0) {
        const sorted = [...intervals].sort((a, b) => a - b);
        const median = sorted[Math.floor(sorted.length / 2)]!;
        setReport({
          frames: sorted.length,
          interval: median,
          hz: median > 0 ? Math.round(1000 / median) : 0,
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
