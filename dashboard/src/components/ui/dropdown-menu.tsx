import * as Menu from "@radix-ui/react-dropdown-menu";
import { CheckIcon } from "lucide-react";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

export const DropdownMenu = Menu.Root;
export const DropdownMenuTrigger = Menu.Trigger;
export const DropdownMenuRadioGroup = Menu.RadioGroup;

export function DropdownMenuContent({
  className,
  sideOffset = 6,
  ...props
}: ComponentProps<typeof Menu.Content>) {
  return (
    <Menu.Portal>
      <Menu.Content
        sideOffset={sideOffset}
        className={cn(
          "z-50 min-w-36 overflow-hidden rounded-lg border bg-popover p-1 text-popover-foreground shadow-lg",
          className,
        )}
        {...props}
      />
    </Menu.Portal>
  );
}

export function DropdownMenuLabel({ className, ...props }: ComponentProps<typeof Menu.Label>) {
  return (
    <Menu.Label
      className={cn("px-2 py-1.5 text-xs font-medium text-muted-foreground", className)}
      {...props}
    />
  );
}

export function DropdownMenuRadioItem({
  className,
  children,
  ...props
}: ComponentProps<typeof Menu.RadioItem>) {
  return (
    <Menu.RadioItem
      className={cn(
        "relative flex cursor-default items-center gap-2 rounded-md py-1.5 ps-8 pe-2 text-sm outline-none select-none data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground [&_svg]:size-4",
        className,
      )}
      {...props}
    >
      <span className="absolute start-2 flex size-4 items-center justify-center">
        <Menu.ItemIndicator>
          <CheckIcon className="text-primary" />
        </Menu.ItemIndicator>
      </span>
      {children}
    </Menu.RadioItem>
  );
}
