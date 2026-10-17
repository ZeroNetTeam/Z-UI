import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createBrowserRouter } from "react-router";
import { RouterProvider } from "react-router/dom";
import { Layout } from "@/components/layout/layout";
import { pages } from "@/components/layout/navigation";
import { TooltipProvider } from "@/components/ui/tooltip";
import { basename } from "@/lib/basename";
import { ThemeProvider } from "@/lib/theme";
import { DashboardPage } from "@/pages/dashboard";
import { NotFoundPage } from "@/pages/not-found";
import { PlaceholderPage } from "@/pages/placeholder";

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } },
});

const router = createBrowserRouter(
  [
    {
      element: <Layout />,
      children: [
        { index: true, element: <DashboardPage /> },
        ...pages
          .filter((item) => item.phase !== undefined)
          .map((item) => ({ path: item.to.slice(1), element: <PlaceholderPage item={item} /> })),
        { path: "*", element: <NotFoundPage /> },
      ],
    },
  ],
  { basename: basename() },
);

export function App() {
  return (
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <TooltipProvider delayDuration={300}>
          <RouterProvider router={router} />
        </TooltipProvider>
      </QueryClientProvider>
    </ThemeProvider>
  );
}
