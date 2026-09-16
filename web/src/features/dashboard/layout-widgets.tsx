// Presentational "layout" widgets — a free-text/heading block and a section divider. They carry no
// data (no frame header, rendered `bare`); in edit mode each exposes lightweight inline controls.

import {SegmentedControl, Select, Textarea} from "@ois/ui";
import {AlignCenter, AlignLeft, AlignRight} from "lucide-react";

import type {DividerWidget, TextWidget} from "./types";

type TextSize = NonNullable<TextWidget["size"]>;
type TextAlign = NonNullable<TextWidget["align"]>;

const SIZE_CLASS: Record<TextSize, string> = {
  sm: "text-sm",
  md: "text-base",
  lg: "text-xl font-bold",
  xl: "text-3xl font-bold tracking-tight",
};
const ALIGN_CLASS: Record<TextAlign, string> = {
  left: "text-left",
  center: "text-center",
  right: "text-right",
};
const SIZES: TextSize[] = ["sm", "md", "lg", "xl"];
const ALIGNS: { id: TextAlign; Icon: typeof AlignLeft }[] = [
  { id: "left", Icon: AlignLeft },
  { id: "center", Icon: AlignCenter },
  { id: "right", Icon: AlignRight },
];

export function TextWidgetView({
  widget,
  editing,
  onChange,
}: {
  widget: TextWidget;
  editing: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  const size = widget.size ?? "md";
  const align = widget.align ?? "left";

  if (!editing) {
    return (
      <div className={"flex h-full flex-col justify-center px-3 " + ALIGN_CLASS[align]}>
        <div className={"whitespace-pre-wrap break-words " + SIZE_CLASS[size]}>
          {widget.content || <span className="text-ink-3">Empty text</span>}
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col gap-1.5 p-2">
      <div className="flex items-center gap-1">
        <Select
          size="sm"
          value={size}
          onChange={(e) => onChange(widget.id, { size: e.target.value })}
          className="h-7 text-xs uppercase"
          aria-label="Text size"
        >
          {SIZES.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </Select>
        <SegmentedControl
          size="sm"
          aria-label="Text alignment"
          value={align}
          onChange={(v) => onChange(widget.id, { align: v })}
          options={ALIGNS.map(({ id, Icon }) => ({ value: id, label: <span className="sr-only">Align {id}</span>, icon: Icon }))}
        />
      </div>
      <Textarea
        value={widget.content}
        onChange={(e) => onChange(widget.id, { content: e.target.value })}
        placeholder="Write a heading or note…"
        className="min-h-0 flex-1 resize-none p-2"
      />
    </div>
  );
}

export function DividerWidgetView({
  widget,
  editing,
  onChange,
}: {
  widget: DividerWidget;
  editing: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  if (widget.orientation === "vertical") {
    return (
      <div className="flex h-full justify-center px-2">
        <span className="h-full w-px bg-line" />
      </div>
    );
  }

  return (
    <div className="flex h-full items-center px-2">
      <div className="flex w-full items-center gap-2 text-xs font-semibold text-ink-3">
        <span className="h-px flex-1 bg-line" />
        {editing ? (
          <input
            value={widget.label ?? ""}
            onChange={(e) => onChange(widget.id, { label: e.target.value })}
            placeholder="Label…"
            className="w-32 rounded-xs border border-line bg-panel-2 px-1.5 py-0.5 text-center text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
        ) : (
          widget.label && <span className="shrink-0">{widget.label}</span>
        )}
        <span className="h-px flex-1 bg-line" />
      </div>
    </div>
  );
}
