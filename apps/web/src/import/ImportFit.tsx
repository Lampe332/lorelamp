import type { MouseEvent as ReactMouseEvent } from "react";
import type { CharInfo, Library } from "../api";
import { openSetting } from "../spot";
import { Icon } from "../ui";
import { t } from "../i18n";

/** After an import: does each new card describe the user the way the persona does? Quiet when it
 *  fits; a link to the character's "Du in dieser Geschichte" when it does not. */
export function ImportFit({ imported, library }: { imported: CharInfo[]; library: Library | null }) {
  // The library's value is newer (another persona may be active by now).
  const fits = imported.map((c) => ({ c, fit: library?.characters.find((x) => x.id === c.id)?.user_check_status ?? c.user_check_status }));
  const name = library?.user_name ? ` ${library.user_name}` : "";
  const off = fits.filter((f) => f.fit !== "ok");
  const okCount = fits.length - off.length;
  const many = fits.length > 1;
  const look = (id: number) => (e: ReactMouseEvent) => {
    e.preventDefault();
    openSetting(`#/char/${id}/settings`, "user-fit");
  };
  return (
    <ul className="import-fit">
      {okCount > 0 && (
        <li className="is-ok">
          <Icon name="check" small />
          <span>
            {!many
              ? t("system.fit.ok", { name })
              : okCount === fits.length
                ? t("system.fit.allOk", { name })
                : t("system.fit.someOk", { n: okCount, name })}
          </span>
        </li>
      )}
      {off.map(({ c, fit }) => (
        <li key={c.id} className={`is-${fit}`}>
          <Icon name={fit === "conflict" ? "alert" : "overlap"} small />
          <span>
            {many && <b>{c.name}: </b>}
            {fit === "conflict" ? t("system.fit.conflict", { name }) : t("system.fit.overlap", { name })}{" "}
            <a href={`#/char/${c.id}/settings`} onClick={look(c.id)} aria-label={t("system.fit.look.aria", { name: c.name })}>
              {t("system.fit.look")}
            </a>
          </span>
        </li>
      ))}
    </ul>
  );
}
