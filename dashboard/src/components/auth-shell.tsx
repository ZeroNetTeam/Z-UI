import type { ReactNode } from "react";
import { Logo } from "@/components/logo";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { HeaderMenus } from "@/components/layout/header";

/** The centred card the sign-in and setup pages share. */
export function AuthShell({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle: string;
  children: ReactNode;
}) {
  return (
    <div className="relative flex min-h-svh items-center justify-center p-4">
      <div className="absolute end-4 top-4 flex gap-1">
        <HeaderMenus />
      </div>
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_top,var(--color-primary)_0%,transparent_60%)] opacity-[0.07]"
      />
      <Card className="relative w-full max-w-sm">
        <CardHeader className="items-center gap-3 pb-4 text-center">
          <Logo className="size-11" />
          <div className="space-y-1.5">
            <CardTitle className="text-lg">{title}</CardTitle>
            <CardDescription className="text-sm">{subtitle}</CardDescription>
          </div>
        </CardHeader>
        <CardContent>{children}</CardContent>
      </Card>
    </div>
  );
}
