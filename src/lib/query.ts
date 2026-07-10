import { QueryClient } from "@tanstack/react-query";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 15_000,
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});

/** Centralised query keys so invalidation stays consistent. */
export const qk = {
  meetings: ["meetings"] as const,
  meeting: (id: string) => ["meeting", id] as const,
  settings: ["settings"] as const,
  recorderStatus: ["recorder", "status"] as const,
  detected: ["detected"] as const,
};
