import {Badge, Card, CardContent} from "@ois/ui";

import {useServiceAccounts} from "@/lib/admin";
import {timeAgo} from "@/lib/time";

export function AdminServiceAccounts() {
  const query = useServiceAccounts();

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">
          Service Accounts
        </h1>
        <p className="text-muted-foreground">
          Machine clients (the Discord bot, tooling) and their roles.
        </p>
      </div>

      <Card>
        <CardContent className="pt-6">
          {query.isError ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Couldn&apos;t load service accounts.
            </p>
          ) : !query.data ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Loading…
            </p>
          ) : query.data.length === 0 ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              No service accounts yet.
            </p>
          ) : (
            <ul className="divide-y">
              {query.data.map((account) => (
                <li
                  key={account.id}
                  className="flex items-center justify-between gap-4 py-3"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="font-medium">{account.name}</span>
                      <Badge
                        variant={
                          account.status === "active" ? "success" : "secondary"
                        }
                      >
                        {account.status}
                      </Badge>
                    </div>
                    <p className="text-xs text-muted-foreground">
                      roles: {account.roles.join(", ") || "none"}
                      {account.last_used_at
                        ? ` · last used ${timeAgo(account.last_used_at)}`
                        : ""}
                    </p>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <p className="text-xs text-muted-foreground">
        Creating, rotating, and disabling credentials from the UI is coming next.
      </p>
    </div>
  );
}
