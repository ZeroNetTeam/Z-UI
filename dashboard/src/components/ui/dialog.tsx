import * as DialogPrimitive from "@radix-ui/react-dialog";
import { XIcon } from "lucide-react";
import type { ComponentProps, ReactNode } from "react";
import { cn } from "@/lib/utils";

export const Dialog = DialogPrimitive.Root;
export const DialogTrigger = DialogPrimitive.Trigger;
export const DialogClose = DialogPrimitive.Close;

export function DialogContent({
  className,
  children,
  title,
  description,
  ...props
}: ComponentProps<typeof DialogPrimitive.Content> & { title: ReactNode; description?: ReactNode }) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="fixed inset-0 z-40 bg-black/60" />
      <DialogPrimitive.Content
        aria-describedby={undefined}
        className={cn(
          "fixed inset-x-0 top-1/2 z-50 mx-auto flex max-h-[92svh] w-[calc(100%-2rem)] max-w-lg -translate-y-1/2 flex-col rounded-xl border bg-card shadow-2xl outline-none",
          className,
        )}
        {...props}
      >
        <div className="flex items-start justify-between gap-4 border-b p-5">
          <div className="grid gap-1">
            <DialogPrimitive.Title className="font-semibold">{title}</DialogPrimitive.Title>
            {description && (
              <DialogPrimitive.Description className="text-sm text-muted-foreground">
                {description}
              </DialogPrimitive.Description>
            )}
          </div>
          <DialogPrimitive.Close className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground">
            <XIcon className="size-4" />
          </DialogPrimitive.Close>
        </div>
        <div className="overflow-y-auto p-5">{children}</div>
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
}
