// Small building blocks every screen shares: the page header, segmented
// radio groups, status badges, callouts and budget meters.

import type { ReactNode } from "react";
import { fmt } from "../format";
import { IconAlert, IconInfo } from "./icons";

export function PageHeader({
  title,
  description,
  crumbs,
  meta,
  actions,
}: {
  title: ReactNode;
  description?: ReactNode;
  /** A way back, above the title. */
  crumbs?: ReactNode;
  /** Facts about the page's subject, under the title. */
  meta?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="page-head">
      <div className="page-head-main">
        {crumbs && <div className="crumbs">{crumbs}</div>}
        <h1 className="page-title">{title}</h1>
        {description && <p className="page-desc">{description}</p>}
        {meta}
      </div>
      {actions && <div className="page-actions">{actions}</div>}
    </div>
  );
}

export interface SegmentOption<T extends string> {
  value: T;
  label: ReactNode;
  /** Muted text after the label. */
  hint?: ReactNode;
  disabled?: boolean;
}

/** A radio group drawn as a segmented control. */
export function Segmented<T extends string>({
  name,
  label,
  value,
  options,
  onChange,
  full,
  className,
}: {
  name: string;
  /** Accessible name for the group. */
  label: string;
  value: T;
  options: SegmentOption<T>[];
  onChange: (value: T) => void;
  /** Stretch to the container, options sharing the width. */
  full?: boolean;
  className?: string;
}) {
  return (
    <fieldset className={["segmented", full ? "full" : "", className ?? ""].filter(Boolean).join(" ")} aria-label={label}>
      {options.map((o) => (
        <label
          key={o.value}
          className={[o.value === value ? "on" : "", o.disabled ? "disabled" : ""].filter(Boolean).join(" ") || undefined}
        >
          <input
            type="radio"
            name={name}
            value={o.value}
            checked={o.value === value}
            disabled={o.disabled}
            onChange={() => onChange(o.value)}
          />
          {o.label}
          {o.hint && <span className="hint">{o.hint}</span>}
        </label>
      ))}
    </fieldset>
  );
}

const STATUS_TONE: Record<string, string> = {
  done: "good",
  running: "accent live",
  pending: "accent",
  failed: "bad",
  cancelled: "",
};

export function StatusBadge({ status }: { status: string }) {
  return <span className={`badge dot ${STATUS_TONE[status] ?? ""}`.trim()}>{status}</span>;
}

/** A boxed note: information, a warning, or an error. */
export function Callout({
  tone = "info",
  title,
  children,
}: {
  tone?: "info" | "warn" | "error";
  title?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className={`callout ${tone}`} role={tone === "error" ? "alert" : undefined}>
      {tone === "info" ? <IconInfo className="callout-icon" /> : <IconAlert className="callout-icon" />}
      <div className="callout-body">
        {title && <div className="callout-title">{title}</div>}
        {children}
      </div>
    </div>
  );
}

/**
 * A used-of-limit meter. The fill is the accent until the limit is passed,
 * then the critical status colour, always with the numbers and a label
 * beside it so the state never rests on colour.
 */
export function Meter({ label, used, limit }: { label: string; used: number; limit: number }) {
  const over = used > limit;
  const fraction = limit > 0 ? Math.min(1, used / limit) : used > 0 ? 1 : 0;
  return (
    <div className={over ? "meter over" : "meter"}>
      <div className="meter-top">
        <span className="meter-label">{label}</span>
        <span className="meter-value">
          {fmt(used)} <span className="muted">/ {fmt(limit)}</span>
        </span>
      </div>
      <div
        className="meter-track"
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={limit}
        aria-valuenow={used}
      >
        <div className="meter-fill" style={{ width: `${fraction * 100}%` }} />
      </div>
      {over && (
        <span className="meter-note">
          <IconAlert size={12} /> {fmt(used - limit)} over the limit
        </span>
      )}
    </div>
  );
}

/** A centred note for an empty list or panel. */
export function Empty({ icon, title, children }: { icon?: ReactNode; title: ReactNode; children?: ReactNode }) {
  return (
    <div className="empty">
      {icon && <div className="empty-icon">{icon}</div>}
      <div className="empty-title">{title}</div>
      {children && <div className="empty-text">{children}</div>}
    </div>
  );
}
