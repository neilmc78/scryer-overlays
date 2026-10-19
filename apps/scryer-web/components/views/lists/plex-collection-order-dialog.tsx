import * as React from "react";
import { ArrowDown, ArrowUp } from "lucide-react";
import { useClient } from "urql";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { IconButton } from "@/components/ui/icon-button";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  movePlexCollectionListMutation,
  movePlexCollectionProviderMutation,
} from "@/lib/graphql/mutations";

import type { PlexCollectionList } from "./plex-collection-setting";

const PROVIDER_NAMES: Record<string, string> = {
  imdb: "IMDb",
  tmdb: "TMDB",
  tvdb: "TVDB",
  anilist: "AniList",
  trakt: "Trakt",
  mal: "MyAnimeList",
  simkl: "Simkl",
  mdblist: "MDBList",
  letterboxd: "Letterboxd",
  plex: "Plex",
  custom: "Custom",
};

type ProviderBlock = {
  provider: string;
  slot: number;
  lists: PlexCollectionList[];
};

/** Mirrored lists grouped into their providers' blocks, in Plex's order. */
function providerBlocks(lists: PlexCollectionList[]): ProviderBlock[] {
  const blocks = new Map<string, ProviderBlock>();
  for (const list of lists) {
    if (!list.enabled || list.slot === null || list.position === null) {
      continue;
    }
    const block = blocks.get(list.provider) ?? {
      provider: list.provider,
      slot: list.slot,
      lists: [],
    };
    block.lists.push(list);
    blocks.set(list.provider, block);
  }
  return [...blocks.values()]
    .sort((a, b) => a.slot - b.slot)
    .map((block) => ({
      ...block,
      lists: [...block.lists].sort((a, b) => (a.position ?? 0) - (b.position ?? 0)),
    }));
}

type PlexCollectionOrderDialogProps = {
  open: boolean;
  lists: PlexCollectionList[];
  onOpenChange: (open: boolean) => void;
  onChanged: () => Promise<void>;
};

/** Reorder mirrored lists inside their provider, and providers as blocks. */
export function PlexCollectionOrderDialog({
  open,
  lists,
  onOpenChange,
  onChanged,
}: PlexCollectionOrderDialogProps) {
  const client = useClient();
  const t = useTranslate();
  const showStatus = useGlobalStatus();
  const [busy, setBusy] = React.useState(false);
  const blocks = providerBlocks(lists);

  const run = async (mutation: string, variables: Record<string, unknown>) => {
    setBusy(true);
    const result = await client.mutation(mutation, variables).toPromise();
    if (result.error) {
      showStatus(result.error.message || t("lists.plexCollection.moveFailed"));
    }
    await onChanged();
    setBusy(false);
  };

  const moveButtons = (
    label: string,
    first: boolean,
    last: boolean,
    move: (up: boolean) => void,
  ) => (
    <div className="flex shrink-0 gap-1">
      <IconButton
        label={`${t("lists.plexCollection.moveUp")}: ${label}`}
        disabled={busy || first}
        onClick={() => move(true)}
      >
        <ArrowUp className="h-3.5 w-3.5" />
      </IconButton>
      <IconButton
        label={`${t("lists.plexCollection.moveDown")}: ${label}`}
        disabled={busy || last}
        onClick={() => move(false)}
      >
        <ArrowDown className="h-3.5 w-3.5" />
      </IconButton>
    </div>
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id="plex-collection-order"
        className="max-h-[90vh] overflow-y-auto sm:max-w-xl"
      >
        <DialogHeader>
          <DialogTitle>{t("lists.plexCollection.orderTitle")}</DialogTitle>
          <DialogDescription>{t("lists.plexCollection.orderHelp")}</DialogDescription>
        </DialogHeader>
        {blocks.length === 0 ? (
          <p className="text-[13px] text-[var(--scry-muted)]">
            {t("lists.plexCollection.orderEmpty")}
          </p>
        ) : (
          <ol className="space-y-3">
            {blocks.map((block) => {
              const name = PROVIDER_NAMES[block.provider] ?? block.provider;
              return (
                <li
                  key={block.provider}
                  className="rounded-md border border-[var(--scry-border3)] p-3"
                >
                  <div className="flex items-center gap-3">
                    <span className="font-mono text-[12px] text-[var(--scry-muted)]">
                      !{block.slot * 1000}_
                    </span>
                    <span className="flex-1 text-[14px] font-medium text-[var(--scry-ink)]">
                      {name}
                    </span>
                    {moveButtons(
                      name,
                      block.slot <= 1,
                      block.slot >= 9,
                      (up) =>
                        void run(movePlexCollectionProviderMutation, {
                          provider: block.provider,
                          up,
                        }),
                    )}
                  </div>
                  <ol className="mt-2 space-y-1">
                    {block.lists.map((list, index) => (
                      <li key={list.subscriptionId} className="flex items-center gap-3 pl-2">
                        <span className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-[var(--scry-ink2)]">
                          {list.sortTitle}
                        </span>
                        {moveButtons(
                          list.name,
                          index === 0,
                          index === block.lists.length - 1,
                          (up) =>
                            void run(movePlexCollectionListMutation, {
                              subscriptionId: list.subscriptionId,
                              up,
                            }),
                        )}
                      </li>
                    ))}
                  </ol>
                </li>
              );
            })}
          </ol>
        )}
      </DialogContent>
    </Dialog>
  );
}
