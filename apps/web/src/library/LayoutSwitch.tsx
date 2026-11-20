// The layout switch of the characters tab: three icon buttons in one track.

import { t } from "../i18n";
import { LIBRARY_LAYOUTS, type LibraryLayoutId } from "./layout";

const PATHS: Record<LibraryLayoutId, string> = {
  poster: "M4.5 4.5h6.5v15H4.5zM13 4.5h6.5v15H13z",
  compact: "M4.5 4.5h4v6h-4zM10 4.5h4v6h-4zM15.5 4.5h4v6h-4zM4.5 13.5h4v6h-4zM10 13.5h4v6h-4zM15.5 13.5h4v6h-4z",
  list: "M4.5 4.5h3.5v3.5H4.5zM11 6.25h8.5M4.5 10.25h3.5v3.5H4.5zM11 12h8.5M4.5 16h3.5v3.5H4.5zM11 17.75h8.5",
};

export function LayoutSwitch({ value, onChange }: { value: LibraryLayoutId; onChange: (id: LibraryLayoutId) => void }) {
  return (
    <div className="lib-seg" role="group" aria-label={t("cards.lib.layout")} data-spot="library-layout">
      {LIBRARY_LAYOUTS.map((l) => {
        const on = l.id === value;
        return (
          <button
            key={l.id}
            type="button"
            aria-pressed={on}
            aria-label={l.name}
            title={`${l.name}: ${l.note}`}
            data-spot-focus={on ? "" : undefined}
            onClick={() => !on && onChange(l.id)}
          >
            <svg className="ic sm" viewBox="0 0 24 24" aria-hidden="true">
              <path d={PATHS[l.id]} />
            </svg>
          </button>
        );
      })}
    </div>
  );
}
