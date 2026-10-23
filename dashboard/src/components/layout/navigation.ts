import {
  ArchiveIcon,
  BellRingIcon,
  BoxesIcon,
  BracesIcon,
  CpuIcon,
  FileKey2Icon,
  HistoryIcon,
  LayoutDashboardIcon,
  LayoutTemplateIcon,
  NetworkIcon,
  ScrollTextIcon,
  ServerIcon,
  SettingsIcon,
  ShieldCheckIcon,
  UsersIcon,
  WaypointsIcon,
  type LucideIcon,
} from "lucide-react";

export interface NavItem {
  /** Route under the panel's base path. */
  to: string;
  /** Key under `nav.` in the translations. */
  label: string;
  icon: LucideIcon;
  /** The roadmap phase that builds it; absent once it exists. */
  phase?: number;
  /** Shown but not yet reachable (arrives after the beta). */
  soon?: boolean;
  /** The permission that opens it (`users.read`); absent for every admin. */
  needs?: string;
  /** Only a sudo admin sees it. */
  sudo?: boolean;
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

export const navigation: NavGroup[] = [
  {
    label: "platform",
    items: [
      { to: "/", label: "dashboard", icon: LayoutDashboardIcon },
      { to: "/users", label: "users", icon: UsersIcon, needs: "users.read" },
      { to: "/inbounds", label: "inbounds", icon: WaypointsIcon, needs: "inbounds.read" },
      { to: "/hosts", label: "hosts", icon: NetworkIcon, needs: "hosts.read" },
      { to: "/groups", label: "groups", icon: BoxesIcon, needs: "groups.read" },
      { to: "/templates", label: "templates", icon: LayoutTemplateIcon, needs: "templates.read" },
      { to: "/nodes", label: "nodes", icon: ServerIcon, soon: true, sudo: true },
      { to: "/admins", label: "admins", icon: ShieldCheckIcon, sudo: true },
    ],
  },
  {
    label: "configuration",
    items: [
      { to: "/certificates", label: "certificates", icon: FileKey2Icon, needs: "certificates.read" },
      { to: "/core", label: "core", icon: CpuIcon, needs: "core.read" },
      { to: "/logs", label: "logs", icon: ScrollTextIcon, needs: "logs.read" },
      { to: "/audit", label: "audit", icon: HistoryIcon, needs: "audit.read" },
      { to: "/notifications", label: "notifications", icon: BellRingIcon, sudo: true },
      { to: "/backup", label: "backup", icon: ArchiveIcon, sudo: true },
      // Not `/api`: that is where the panel's own API lives.
      { to: "/developer", label: "api", icon: BracesIcon },
      { to: "/settings", label: "settings", icon: SettingsIcon },
    ],
  },
];

/** Every page that exists so far or is planned, for the router. */
export const pages = navigation.flatMap((group) => group.items);
