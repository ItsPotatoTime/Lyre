import { useEffect, useRef, useState } from "react";

/**
 * Explorer's reveal highlight: a soft wash that follows the pointer across the
 * control being pointed at.
 *
 * Delegated on purpose. Attaching a pointermove listener per row would cost
 * more than the effect is worth in a 50,000 row list, and the variable is
 * written only on the hovered element, so the style recalc never touches its
 * siblings.
 */
export function useReveal() {
  useEffect(() => {
    let active: HTMLElement | null = null;

    const clear = (el: HTMLElement) => {
      el.style.removeProperty("--mx");
      el.style.removeProperty("--my");
    };

    const onMove = (event: PointerEvent) => {
      const target = (event.target as Element | null)?.closest?.(
        "[data-reveal]",
      ) as HTMLElement | null;
      if (target !== active) {
        if (active) clear(active);
        active = target;
      }
      if (!target) return;
      const rect = target.getBoundingClientRect();
      target.style.setProperty("--mx", `${event.clientX - rect.left}px`);
      target.style.setProperty("--my", `${event.clientY - rect.top}px`);
    };

    const onLeave = () => {
      if (active) clear(active);
      active = null;
    };

    window.addEventListener("pointermove", onMove, { passive: true });
    window.addEventListener("pointerleave", onLeave);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerleave", onLeave);
    };
  }, []);
}

/**
 * Windowed rendering bounds for a scroll container.
 *
 * Only the rows inside the viewport (plus a small overscan) are returned, so
 * the DOM size is a function of the window height rather than of the
 * directory. Measurements are coalesced into one animation frame, which keeps
 * a fast trackpad fling from queueing a render per scroll event.
 */
export function useVirtual(
  count: number,
  rowHeight: number,
  overscan = 6,
) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [bounds, setBounds] = useState({ start: 0, end: 40, height: 640, width: 0 });
  // The row count changes on every streamed batch. Keeping it in a ref means
  // the listener and observer setup below runs once per layout change instead
  // of being torn down and rebuilt dozens of times during one scan.
  const countRef = useRef(count);
  countRef.current = count;
  const measureRef = useRef<() => void>(() => {});

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let frame = 0;

    const measure = () => {
      frame = 0;
      const height = el.clientHeight;
      const width = el.clientWidth;
      const top = el.scrollTop;
      const start = Math.max(0, Math.floor(top / rowHeight) - overscan);
      const end = Math.min(
        countRef.current,
        Math.ceil((top + height) / rowHeight) + overscan,
      );
      setBounds((prev) =>
        prev.start === start &&
        prev.end === end &&
        prev.height === height &&
        prev.width === width
          ? prev
          : { start, end, height, width },
      );
    };
    measureRef.current = measure;

    const onScroll = () => {
      if (frame === 0) frame = requestAnimationFrame(measure);
    };

    measure();
    el.addEventListener("scroll", onScroll, { passive: true });
    const observer = new ResizeObserver(onScroll);
    observer.observe(el);
    return () => {
      el.removeEventListener("scroll", onScroll);
      observer.disconnect();
      if (frame !== 0) cancelAnimationFrame(frame);
    };
  }, [rowHeight, overscan]);

  // More rows can mean more visible rows, so re-measure when the count moves.
  useEffect(() => {
    measureRef.current();
  }, [count]);

  return { ref, bounds } as const;
}

/** True only after `delay` ms of the flag being set, to avoid flashing a
 *  loading state for a scan that finishes in two frames. */
export function useDelayed(flag: boolean, delay = 140): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!flag) {
      setShown(false);
      return;
    }
    const timer = setTimeout(() => setShown(true), delay);
    return () => clearTimeout(timer);
  }, [flag, delay]);
  return shown;
}
