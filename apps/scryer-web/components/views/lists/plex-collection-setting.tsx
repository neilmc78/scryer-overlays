import * as React from "react";
import { ListOrdered, RefreshCw } from "lucide-react";
import { useClient } from "urql";

import { LabeledFieldset } from "@/components/common/labeled-fieldset";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  setPlexCollectionListMutation,
  syncPlexCollectionsMutation,
} from "@/lib/graphql/mutations";
import { plexCollectionListsQuery } from "@/lib/graphql/queries";

import { PlexCollectionOrderDialog } from "./plex-collection-order-dialog";

export type PlexCollectionList = {
  subscriptionId: string;
  name: string;
  enabled: boolean;
  titles: number;
  pushedItems: number;
  failedItems: number;
  lastError: string | null;
  provider: string;
  slot: number | null;
  position: number | null;
  sortTitle: string | null;
  posterEnabled: boolean;
  posterBackground: string;
  posterText: string | null;
  posterLogo: boolean;
  posterLogoAvailable: boolean;
  posterLogoSource: "provider" | "custom" | "none";
  posterCustomLogo: boolean;
};

type PlexCollectionSettingProps = {
  subscriptionId: string;
};

/**
 * Whether a public list is mirrored into Plex as a collection named after
 * it. Loads its own state so the list panel only has to place it.
 */
export function PlexCollectionSetting({ subscriptionId }: PlexCollectionSettingProps) {
  const client = useClient();
  const t = useTranslate();
  const showStatus = useGlobalStatus();
  const [lists, setLists] = React.useState<PlexCollectionList[]>([]);
  const [orderOpen, setOrderOpen] = React.useState(false);
  const list = lists.find((entry) => entry.subscriptionId === subscriptionId) ?? null;
  const [busy, setBusy] = React.useState(false);

  const reload = React.useCallback(async () => {
    const result = await client
      .query(plexCollectionListsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    setLists((result.data?.plexCollectionLists ?? []) as PlexCollectionList[]);
  }, [client]);

  React.useEffect(() => {
    void reload();
  }, [reload]);

  if (!list) {
    return null;
  }

  const setEnabled = async (enabled: boolean) => {
    setBusy(true);
    const result = await client
      .mutation(setPlexCollectionListMutation, { subscriptionId, enabled })
      .toPromise();
    if (result.error) {
      showStatus(result.error.message || t("lists.plexCollection.saveFailed"));
    }
    await reload();
    setBusy(false);
  };

  const syncNow = async () => {
    setBusy(true);
    const result = await client.mutation(syncPlexCollectionsMutation, {}).toPromise();
    showStatus(
      result.error
        ? result.error.message || t("lists.plexCollection.syncFailed")
        : t("lists.plexCollection.syncQueued"),
    );
    setBusy(false);
  };

  return (
    <LabeledFieldset label={t("lists.plexCollection.heading")}>
      <label className="flex items-center gap-3 text-[13px] text-[var(--scry-ink2)]">
        <Switch
          id="list-detail-plex-collection"
          checked={list.enabled}
          disabled={busy}
          onCheckedChange={(checked) => void setEnabled(checked)}
        />
        {t("lists.plexCollection.toggle")}
      </label>
      <p className="text-[12.5px] text-[var(--scry-muted)]">{t("lists.plexCollection.help")}</p>
      {list.enabled ? (
        <p className="text-[12.5px] text-[var(--scry-ink2)]">
          {t("lists.plexCollection.status", { pushed: list.pushedItems, titles: list.titles })}
        </p>
      ) : null}
      {list.failedItems > 0 ? (
        <p className="text-[12.5px] text-[var(--scry-danger-text)]">
          {t("lists.plexCollection.failed", {
            count: list.failedItems,
            error: list.lastError ?? "",
          })}
        </p>
      ) : null}
      {list.enabled && list.sortTitle ? (
        <p className="font-mono text-[12.5px] text-[var(--scry-ink2)]">
          {t("lists.plexCollection.sortTitle", { sortTitle: list.sortTitle })}
        </p>
      ) : null}
      {list.enabled ? (
        <div className="flex flex-wrap gap-2">
          <Button
            id="list-detail-plex-collection-sync"
            type="button"
            size="xs"
            variant="outline"
            onClick={() => void syncNow()}
            disabled={busy}
          >
            <RefreshCw className="h-3.5 w-3.5" />
            {t("lists.plexCollection.syncNow")}
          </Button>
          <Button
            id="list-detail-plex-collection-order"
            type="button"
            size="xs"
            variant="outline"
            onClick={() => setOrderOpen(true)}
            disabled={busy}
          >
            <ListOrdered className="h-3.5 w-3.5" />
            {t("lists.plexCollection.order")}
          </Button>
        </div>
      ) : null}
      <PlexCollectionOrderDialog
        open={orderOpen}
        lists={lists}
        onOpenChange={setOrderOpen}
        onChanged={reload}
      />
    </LabeledFieldset>
  );
}
