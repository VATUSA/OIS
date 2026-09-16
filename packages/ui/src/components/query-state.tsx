import * as React from "react";
import {AlertCircle, Inbox, Loader2} from "lucide-react";

import {cn} from "../lib/utils";
import {Button} from "./button";

/** A quiet centred notice — the one empty / loading / error line. */
export function EmptyState({
  icon: Icon = Inbox,
  title,
  children,
  action,
  className,
}: {
  icon?: React.ComponentType<{ className?: string }>;
  title?: React.ReactNode;
  children?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex flex-col items-center justify-center gap-2 px-4 py-8 text-center", className)}>
      <Icon className="size-5 text-ink-3" />
      {title && <p className="text-sm font-semibold text-ink">{title}</p>}
      {children && <p className="max-w-prose text-sm text-ink-2">{children}</p>}
      {action}
    </div>
  );
}

const Spinner = ({ className }: { className?: string }) => <Loader2 className={cn("animate-spin", className)} />;

/**
 * Render a query's three non-data states consistently, then the data. Pass the react-query flags;
 * `isEmpty` is evaluated only once loaded.
 *
 *   <QueryState isLoading={q.isLoading} isError={q.isError} isEmpty={!q.data?.length} empty="No stops.">
 *     <Table … />
 *   </QueryState>
 */
export function QueryState({
  isLoading,
  isError,
  isEmpty = false,
  loading = "Loading…",
  error = "Couldn't load this.",
  empty = "Nothing here yet.",
  onRetry,
  className,
  children,
}: {
  isLoading?: boolean;
  isError?: boolean;
  isEmpty?: boolean;
  loading?: React.ReactNode;
  error?: React.ReactNode;
  empty?: React.ReactNode;
  onRetry?: () => void;
  className?: string;
  children?: React.ReactNode;
}) {
  if (isError) {
    return (
      <EmptyState
        icon={AlertCircle}
        className={className}
        action={
          onRetry && (
            <Button size="sm" variant="secondary" onClick={onRetry}>
              Retry
            </Button>
          )
        }
      >
        {error}
      </EmptyState>
    );
  }
  if (isLoading) {
    return (
      <EmptyState icon={Spinner} className={className}>
        {loading}
      </EmptyState>
    );
  }
  if (isEmpty) {
    return <EmptyState className={className}>{empty}</EmptyState>;
  }
  return <>{children}</>;
}
