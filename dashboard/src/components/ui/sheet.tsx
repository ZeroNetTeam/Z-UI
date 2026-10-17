import * as Dialog from "@radix-ui/react-dialog";
import type { ComponentProps } from "react";
import { cn } from "@/lib/utils";

/** A panel that slides in from the reading start: the sidebar on phones. */
export const Sheet = Dialog.Root;
export const SheetTrigger = Dialog.Trigger;
export const SheetTitle = Dialog.Title;

export function SheetContent({ className, children, ...props }: ComponentProps<typeof Dialog.Content>) {
  return (
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-40 bg-black/60" />
      <Dialog.Content
        aria-describedby={undefined}
        className={cn(
          "fixed inset-y-0 start-0 z-50 flex w-72 max-w-[85vw] flex-col border-e bg-sidebar shadow-xl outline-none",
          className,
        )}
        {...props}
      >
        {children}
      </Dialog.Content>
    </Dialog.Portal>
  );
}
