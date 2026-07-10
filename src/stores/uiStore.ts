import { create } from "zustand";

export type SortMode = "recent" | "oldest" | "longest" | "title";
export type ViewFilter = "all" | "starred" | "bookmarked" | "locked";

interface UIState {
  /** Multi-select in the meetings list. */
  selected: Set<string>;
  toggleSelected: (id: string) => void;
  clearSelected: () => void;
  isSelected: (id: string) => boolean;

  search: string;
  setSearch: (v: string) => void;

  sort: SortMode;
  setSort: (v: SortMode) => void;

  filter: ViewFilter;
  setFilter: (v: ViewFilter) => void;
}

export const useUIStore = create<UIState>((set, get) => ({
  selected: new Set<string>(),
  toggleSelected: (id) =>
    set((s) => {
      const next = new Set(s.selected);
      next.has(id) ? next.delete(id) : next.add(id);
      return { selected: next };
    }),
  clearSelected: () => set({ selected: new Set() }),
  isSelected: (id) => get().selected.has(id),

  search: "",
  setSearch: (v) => set({ search: v }),

  sort: "recent",
  setSort: (v) => set({ sort: v }),

  filter: "all",
  setFilter: (v) => set({ filter: v }),
}));
