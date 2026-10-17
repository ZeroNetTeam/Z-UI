import {
  BoxesIcon,
  CpuIcon,
  LayoutDashboardIcon,
  LayoutTemplateIcon,
  NetworkIcon,
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
      { to: "/users", label: "users", icon: UsersIcon, phase: 4 },
      { to: "/inbounds", label: "inbounds", icon: WaypointsIcon, phase: 3 },
      { to: "/hosts", label: "hosts", icon: NetworkIcon, phase: 3 },
      { to: "/groups", label: "groups", icon: BoxesIcon, phase: 4 },
      { to: "/templates", label: "templates", icon: LayoutTemplateIcon, phase: 4 },
      { to: "/nodes", label: "nodes", icon: ServerIcon, soon: true },
      { to: "/admins", label: "admins", icon: ShieldCheckIcon, phase: 8 },
    ],
  },
  {
    label: "configuration",
    items: [
      { to: "/core", label: "core", icon: CpuIcon, phase: 7 },
      { to: "/settings", label: "settings", icon: SettingsIcon, phase: 2 },
    ],
  },
];

/** Every page that exists so far or is planned, for the router. */
export const pages = navigation.flatMap((group) => group.items);
