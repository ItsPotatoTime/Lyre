import { useCallback, useEffect, useState } from "react";

export type ThemePref = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

const KEY = "lyre.theme";

function systemTheme(): ResolvedTheme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

export function resolveTheme(pref: ThemePref): ResolvedTheme {
  return pref === "system" ? systemTheme() : pref;
}

export function applyTheme(pref: ThemePref): ResolvedTheme {
  const resolved = resolveTheme(pref);
  document.documentElement.dataset["theme"] = resolved;
  return resolved;
}

function loadPref(): ThemePref {
  try {
    const stored = localStorage.getItem(KEY);
    if (stored === "light" || stored === "dark" || stored === "system") {
      return stored;
    }
  } catch {
    // Private mode or a locked-down webview: fall through to the system.
  }
  return "system";
}

export function useTheme(): {
  pref: ThemePref;
  resolved: ResolvedTheme;
  setPref: (next: ThemePref) => void;
  cycle: () => void;
} {
  const [pref, setPrefState] = useState<ThemePref>(loadPref);
  const [resolved, setResolved] = useState<ResolvedTheme>(() =>
    resolveTheme(loadPref()),
  );

  useEffect(() => {
    setResolved(applyTheme(pref));
    try {
      localStorage.setItem(KEY, pref);
    } catch {
      // Not fatal: the preference just does not survive the session.
    }
  }, [pref]);

  useEffect(() => {
    if (pref !== "system") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setResolved(applyTheme("system"));
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [pref]);

  const setPref = useCallback((next: ThemePref) => setPrefState(next), []);
  const cycle = useCallback(
    () =>
      setPrefState((current) =>
        current === "system" ? "light" : current === "light" ? "dark" : "system",
      ),
    [],
  );

  return { pref, resolved, setPref, cycle };
}
