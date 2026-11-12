import { SearchIcon, XIcon } from "lucide-react";
import { type KeyboardEvent, type RefObject, useId, useState } from "react";
import { cn } from "@/shared/lib/cn";
import { Hint } from "@/shared/ui/tooltip";
import type { QueryNote } from "./query/parse";
import { type Facets, suggest } from "./query/suggest";

/**
 * The filter: one line in the query language. Completions for keys and values appear under it as it
 * is typed; what the filter could not use is said beneath, never silently dropped.
 */
export function FilterBar({
  inputRef,
  value,
  onChange,
  notes,
  facets,
}: {
  inputRef: RefObject<HTMLInputElement | null>;
  value: string;
  onChange: (value: string) => void;
  notes: QueryNote[];
  facets: Facets;
}) {
  const listId = useId();
  const notesId = useId();
  const [caret, setCaret] = useState(value.length);
  const [open, setOpen] = useState(false);
  const [highlighted, setHighlighted] = useState(0);
  const suggestions = open ? suggest(value, caret, facets) : [];
  const active = suggestions[Math.min(highlighted, suggestions.length - 1)];

  const accept = (index: number) => {
    const suggestion = suggestions[index];
    if (!suggestion) return;
    onChange(suggestion.query);
    setCaret(suggestion.caret);
    setHighlighted(0);
    // The caret can only be placed once the input holds the new text.
    requestAnimationFrame(() => inputRef.current?.setSelectionRange(suggestion.caret, suggestion.caret));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (suggestions.length > 0) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const step = event.key === "ArrowDown" ? 1 : -1;
        setHighlighted((highlighted + step + suggestions.length) % suggestions.length);
        return;
      }
      if (event.key === "Tab" || event.key === "Enter") {
        event.preventDefault();
        accept(Math.min(highlighted, suggestions.length - 1));
        return;
      }
      if (event.key === "Escape") {
        // The first Escape only closes the list; the workbench must not take it for "close the details".
        event.stopPropagation();
        setOpen(false);
        return;
      }
    }
    if (event.key === "Escape" && value !== "") {
      event.stopPropagation();
      onChange("");
    }
  };

  return (
    <div className="relative min-w-0 flex-1">
      <SearchIcon
        aria-hidden="true"
        className="pointer-events-none absolute top-[7px] left-2.5 size-3.5 text-subtle-foreground"
      />
      <input
        ref={inputRef}
        type="text"
        role="combobox"
        aria-label="Filter requests"
        aria-expanded={suggestions.length > 0}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={active ? `${listId}-${suggestions.indexOf(active)}` : undefined}
        aria-describedby={notes.length > 0 ? notesId : undefined}
        spellCheck={false}
        autoComplete="off"
        placeholder="Filter: text, cat:bid, is:late, -host:example.com, /regex/"
        value={value}
        onChange={(event) => {
          onChange(event.target.value);
          setCaret(event.target.selectionStart ?? event.target.value.length);
          setOpen(true);
          setHighlighted(0);
        }}
        onSelect={(event) => setCaret(event.currentTarget.selectionStart ?? value.length)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onKeyDown={onKeyDown}
        className="h-7 w-full rounded-md border border-input bg-background pr-7 pl-8 font-mono text-xs outline-none placeholder:font-sans placeholder:text-subtle-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40"
      />
      {value ? (
        <Hint label="Clear the filter">
          <button
            type="button"
            aria-label="Clear the filter"
            onClick={() => {
              onChange("");
              inputRef.current?.focus();
            }}
            className="absolute top-1 right-1 flex size-5 items-center justify-center rounded-sm text-subtle-foreground hover:bg-accent hover:text-foreground"
          >
            <XIcon className="size-3.5" />
          </button>
        </Hint>
      ) : null}
      <div
        id={listId}
        role="listbox"
        aria-label="Filter completions"
        className={cn(
          "absolute top-8 left-0 z-30 min-w-56 rounded-lg border bg-popover p-1 shadow-pop",
          suggestions.length === 0 && "hidden",
        )}
      >
        {suggestions.map((suggestion, index) => (
          // biome-ignore lint/a11y/useKeyWithClickEvents: keys are handled on the input that owns the list
          <div
            key={suggestion.label}
            id={`${listId}-${index}`}
            role="option"
            aria-selected={suggestion === active}
            // Focus stays in the input, which names the highlighted option as its active descendant.
            tabIndex={-1}
            // Before the input loses focus, so the click lands.
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => accept(index)}
            className={cn("rounded-md px-2 py-1 font-mono text-xs", suggestion === active && "bg-accent")}
          >
            {suggestion.label}
          </div>
        ))}
      </div>
      {notes.length > 0 ? (
        <p
          id={notesId}
          className="absolute top-8 left-0 z-20 max-w-full truncate rounded-md border border-amber-line bg-amber-soft px-2 py-0.5 text-[11px] text-amber-fg"
        >
          {notes.map((note) => note.message).join(" · ")}
        </p>
      ) : null}
    </div>
  );
}
