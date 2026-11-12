import {
  CheckCheckIcon,
  CircleDollarSignIcon,
  HourglassIcon,
  type LucideIcon,
  MinusIcon,
  TimerOffIcon,
  TrophyIcon,
  XIcon,
} from "lucide-react";
import type { BidStatus } from "@/bindings";

/** One visual language for bid status everywhere: a mark colour, an icon and (from the vocabulary) a label. */
export const BID_STATUS_STYLE: Record<BidStatus, { icon: LucideIcon; bar: string; iconTone: string }> = {
  rendered: { icon: CheckCheckIcon, bar: "bg-green", iconTone: "text-green-fg" },
  won: { icon: TrophyIcon, bar: "bg-violet", iconTone: "text-violet-fg" },
  bid: { icon: CircleDollarSignIcon, bar: "bg-blue", iconTone: "text-blue-fg" },
  noBid: { icon: MinusIcon, bar: "bg-gray/55", iconTone: "text-subtle-foreground" },
  timeout: { icon: TimerOffIcon, bar: "bg-amber hatch", iconTone: "text-amber-fg" },
  error: { icon: XIcon, bar: "bg-red", iconTone: "text-red-fg" },
  pending: { icon: HourglassIcon, bar: "bg-gray/35 hatch", iconTone: "text-subtle-foreground" },
};
