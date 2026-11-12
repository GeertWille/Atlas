import { Toaster as Sonner, type ToasterProps } from "sonner";

function Toaster({ theme }: { theme: NonNullable<ToasterProps["theme"]> }) {
  return (
    <Sonner
      theme={theme}
      position="bottom-right"
      closeButton
      toastOptions={{
        classNames: {
          toast: "!rounded-lg !border-border !bg-popover !text-popover-foreground !shadow-pop !font-sans",
          description: "!text-muted-foreground",
          actionButton: "!bg-primary !text-primary-foreground",
        },
      }}
    />
  );
}

export { Toaster };
