import type {ReactNode} from "react";

/** A planning section's optional 20/700 title, a one-line description, and trailing actions. Inside the
 * event page's tabs the tab already names the section, so those omit `title`. */
export function SectionHeader({
  title,
  description,
  actions,
}: {
  title?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-end justify-between gap-3">
      <div className="flex min-w-0 flex-col gap-1">
        {title && <h2 className="text-xl font-bold">{title}</h2>}
        {description && <p className="max-w-3xl text-sm text-ink-2">{description}</p>}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}
