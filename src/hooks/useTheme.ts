import { useEffect } from "react";
import { useSettings } from "./useMeetings";

/** Applies the persisted theme to <html> and follows the OS when set to "system". */
export function useApplyTheme() {
  const { data: settings } = useSettings();
  const theme = settings?.theme ?? "light";

  useEffect(() => {
    const root = document.documentElement;
    const media = window.matchMedia("(prefers-color-scheme: dark)");

    const apply = () => {
      const dark = theme === "dark" || (theme === "system" && media.matches);
      root.classList.toggle("dark", dark);
    };

    apply();
    if (theme === "system") {
      media.addEventListener("change", apply);
      return () => media.removeEventListener("change", apply);
    }
  }, [theme]);
}
