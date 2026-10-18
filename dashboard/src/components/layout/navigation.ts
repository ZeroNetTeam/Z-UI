import {
  BoxesIcon,
  CpuIcon,
  FileKey2Icon,
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
      { to: "/users", label: "users", icon: UsersIcon },
      { to: "/inbounds", label: "inbounds", icon: WaypointsIcon },
      { to: "/hosts", label: "hosts", icon: NetworkIcon },
      { to: "/groups", label: "groups", icon: BoxesIcon },
      { to: "/templates", label: "templates", icon: LayoutTemplateIcon },
      { to: "/nodes", label: "nodes", icon: ServerIcon, soon: true },
      { to: "/admins", label: "admins", icon: ShieldCheckIcon, phase: 8 },
    ],
  },
  {
    label: "configuration",
    items: [
      { to: "/certificates", label: "certificates", icon: FileKey2Icon },
      { to: "/core", label: "core", icon: CpuIcon, phase: 7 },
      { to: "/settings", label: "settings", icon: SettingsIcon },
    ],
  },
];

/** Every page that exists so far or is planned, for the router. */
export const pages = navigation.flatMap((group) => group.items);
