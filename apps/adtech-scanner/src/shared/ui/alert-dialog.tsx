import { AlertDialog as AlertDialogPrimitive } from "radix-ui";
import type { ComponentProps } from "react";
import { cn } from "@/shared/lib/cn";
import { buttonVariants } from "./button";
import { modalClasses } from "./dialog";

const AlertDialog = AlertDialogPrimitive.Root;
const AlertDialogTrigger = AlertDialogPrimitive.Trigger;

function AlertDialogContent({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Content>) {
  return (
    <AlertDialogPrimitive.Portal>
      <AlertDialogPrimitive.Overlay className={modalClasses.overlay} />
      <AlertDialogPrimitive.Content
        data-slot="alert-dialog-content"
        className={cn(modalClasses.content, "max-w-sm", className)}
        {...props}
      />
    </AlertDialogPrimitive.Portal>
  );
}

function AlertDialogHeader({ className, ...props }: ComponentProps<"div">) {
  return <div className={cn("flex flex-col gap-1.5", className)} {...props} />;
}

function AlertDialogFooter({ className, ...props }: ComponentProps<"div">) {
  return <div className={cn("flex justify-end gap-2", className)} {...props} />;
}

function AlertDialogTitle({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Title>) {
  return <AlertDialogPrimitive.Title className={cn(modalClasses.title, className)} {...props} />;
}

function AlertDialogDescription({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Description>) {
  return <AlertDialogPrimitive.Description className={cn(modalClasses.description, className)} {...props} />;
}

function AlertDialogAction({
  className,
  destructive = false,
  ...props
}: ComponentProps<typeof AlertDialogPrimitive.Action> & { destructive?: boolean }) {
  return (
    <AlertDialogPrimitive.Action
      className={cn(
        buttonVariants({ variant: destructive ? "destructive" : "default" }),
        "pointer-input:outline-none",
        className,
      )}
      {...props}
    />
  );
}

function AlertDialogCancel({ className, ...props }: ComponentProps<typeof AlertDialogPrimitive.Cancel>) {
  return (
    <AlertDialogPrimitive.Cancel
      className={cn(buttonVariants({ variant: "outline" }), "pointer-input:outline-none", className)}
      {...props}
    />
  );
}

export {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
};
