import { useCallback, useEffect, useState } from "react";

type Theme = "light" | "dark";

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(() =>
    document.documentElement.classList.contains("dark") ? "dark" : "light"
  );

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    try {
      localStorage.setItem("irecon-theme", theme);
    } catch {
      // storage can be unavailable; the class on the document still applies
    }
  }, [theme]);

  const toggle = useCallback(
    () => setTheme((current) => (current === "dark" ? "light" : "dark")),
    []
  );

  return { theme, toggle };
}
