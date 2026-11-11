import { QueryClient } from "@tanstack/react-query";

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        // The backend is local and pushes changes as events; refetching on focus adds nothing.
        staleTime: 30_000,
        refetchOnWindowFocus: false,
        retry: false,
      },
    },
  });
}
