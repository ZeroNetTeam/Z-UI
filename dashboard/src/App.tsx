import { MutationCache, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { createBrowserRouter } from "react-router";
import { RouterProvider } from "react-router/dom";
import { Layout } from "@/components/layout/layout";
import { pages } from "@/components/layout/navigation";
import { Toaster } from "@/components/toaster";
import { TooltipProvider } from "@/components/ui/tooltip";
import { basename } from "@/lib/basename";
import { ThemeProvider } from "@/lib/theme";
import { Gate } from "@/components/gate";
import { RequireAdmin } from "@/components/require-admin";
import i18n from "@/i18n";
import { setUnauthorizedHandler } from "@/lib/api";
import { errorText } from "@/lib/errors";
import { toast } from "@/lib/toast";
import { authKey } from "@/lib/auth";
import { AdminsPage } from "@/pages/admins";
import { AuditPage } from "@/pages/audit";
import { BackupPage } from "@/pages/backup";
import { MigratePage } from "@/pages/migrate";
import { CertificatesPage } from "@/pages/certificates";
import { CorePage } from "@/pages/core";
import { DashboardPage } from "@/pages/dashboard";
import { DeveloperPage } from "@/pages/developer";
import { GroupsPage } from "@/pages/groups";
import { HostsPage } from "@/pages/hosts";
import { InboundsPage } from "@/pages/inbounds";
import { LoginPage } from "@/pages/login";
import { LogsPage } from "@/pages/logs";
import { NotificationsPage } from "@/pages/notifications";
import { SettingsPage } from "@/pages/settings";
import { InvitePage } from "@/pages/invite";
import { SetupPage } from "@/pages/setup";
import { TemplatesPage } from "@/pages/templates";
import { UsersPage } from "@/pages/users";
import { NotFoundPage } from "@/pages/not-found";
import { PlaceholderPage } from "@/pages/placeholder";

/**
 * What a mutation says when it ends, so nothing the admin does is silent:
 * a failure is told in a toast whatever the page also shows, and a success is
 * told too, in the words the mutation gave (`meta.success`, an i18n key) or
 * as "Done". `meta.quiet` is for the ones that show their own result.
 */
const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } },
  mutationCache: new MutationCache({
    onSuccess: (_data, _variables, _context, mutation) => {
      if (mutation.meta?.quiet) return;
      const key = mutation.meta?.success;
      toast(i18n.t(typeof key === "string" ? key : "toast.done"), "success");
    },
    onError: (error, _variables, _context, mutation) => {
      if (mutation.meta?.quiet) return;
      toast(errorText(error, i18n.t.bind(i18n), i18n.language), "error");
    },
  }),
});

// A session that ends while the page is open sends it back to sign in.
setUnauthorizedHandler(() => void queryClient.invalidateQueries({ queryKey: authKey }));

/** The page behind each place in the navigation. */
const screens: Record<string, ReactNode> = {
  "/settings": <SettingsPage />,
  "/users": <UsersPage />,
  "/inbounds": <InboundsPage />,
  "/groups": <GroupsPage />,
  "/templates": <TemplatesPage />,
  "/hosts": <HostsPage />,
  "/certificates": <CertificatesPage />,
  "/logs": <LogsPage />,
  "/core": <CorePage />,
  "/admins": <AdminsPage />,
  "/audit": <AuditPage />,
  "/notifications": <NotificationsPage />,
  "/backup": <BackupPage />,
  "/migrate": <MigratePage />,
  "/developer": <DeveloperPage />,
};

const router = createBrowserRouter(
  [
    { path: "login", element: <LoginPage /> },
    { path: "setup", element: <SetupPage /> },
    { path: "invite/:token", element: <InvitePage /> },
    {
      element: (
        <RequireAdmin>
          <Layout />
        </RequireAdmin>
      ),
      children: [
        { index: true, element: <DashboardPage /> },
        // Each page opens only for an admin the navigation says may.
        ...pages
          .filter((item) => screens[item.to] || item.phase !== undefined)
          .map((item) => ({
            path: item.to.slice(1),
            element: <Gate to={item.to}>{screens[item.to] ?? <PlaceholderPage item={item} />}</Gate>,
          })),
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
          <Toaster />
        </TooltipProvider>
      </QueryClientProvider>
    </ThemeProvider>
  );
}
