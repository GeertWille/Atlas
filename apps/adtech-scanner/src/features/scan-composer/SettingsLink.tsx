import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";

export function SettingsLink({ children }: { children: ReactNode }) {
  return (
    <Link to="/settings" className="font-medium text-primary-text hover:underline">
      {children}
    </Link>
  );
}
