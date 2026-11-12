import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";

export function SettingsLink({ children }: { children: ReactNode }) {
  return (
    <Link to="/settings" className="font-medium whitespace-nowrap text-primary-text hover:underline">
      {children}
    </Link>
  );
}
