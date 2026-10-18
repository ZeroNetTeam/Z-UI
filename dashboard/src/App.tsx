import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createBrowserRouter } from "react-router";
import { RouterProvider } from "react-router/dom";
import { Layout } from "@/components/layout/layout";
import { pages } from "@/components/layout/navigation";
import { TooltipProvider } from "@/components/ui/tooltip";
import { basename } from "@/lib/basename";
import { ThemeProvider } from "@/lib/theme";
import { RequireAdmin } from "@/components/require-admin";
import { setUnauthorizedHandler } from "@/lib/api";
import { authKey } from "@/lib/auth";
import { CertificatesPage } from "@/pages/certificates";
import { DashboardPage } from "@/pages/dashboard";
import { GroupsPage } from "@/pages/groups";
import { HostsPage } from "@/pages/hosts";
import { InboundsPage } from "@/pages/inbounds";
import { LoginPage } from "@/pages/login";
import { SettingsPage } from "@/pages/settings";
import { SetupPage } from "@/pages/setup";
import { TemplatesPage } from "@/pages/templates";
import { UsersPage } from "@/pages/users";
import { NotFoundPage } from "@/pages/not-found";
import { PlaceholderPage } from "@/pages/placeholder";

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } },
});

// A session that ends while the page is open sends it back to sign in.
setUnauthorizedHandler(() => void queryClient.invalidateQueries({ queryKey: authKey }));

const router = createBrowserRouter(
  [
    { path: "login", element: <LoginPage /> },
    { path: "setup", element: <SetupPage /> },
    {
      element: (
        <RequireAdmin>
          <Layout />
        </RequireAdmin>
      ),
      children: [
        { index: true, element: <DashboardPage /> },
        { path: "settings", element: <SettingsPage /> },
        { path: "users", element: <UsersPage /> },
        { path: "inbounds", element: <InboundsPage /> },
        { path: "groups", element: <GroupsPage /> },
        { path: "templates", element: <TemplatesPage /> },
        { path: "hosts", element: <HostsPage /> },
        { path: "certificates", element: <CertificatesPage /> },
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
