import {
  CopyIcon,
  FolderOpenIcon,
  ImageIcon,
  Trash2Icon,
  UploadIcon,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, type Asset } from "~/lib/api";
import { useFileDrop } from "~/lib/useFileDrop";
import { cn, formatBytes } from "~/lib/utils";
import { useStudio } from "~/state/studio";
import { useProjectDoc } from "~/state/project";
import { Button } from "../ui/button";
import { confirmAction } from "../ui/dialog";
import { Spinner } from "../ui/input";
import { toast } from "../ui/toast";

export function AssetsPanel({ projectId }: { projectId: string }) {
  const workspace = useProjectDoc((state) => state.workspace);
  const [assets, setAssets] = useState<Asset[]>([]);
  const [loading, setLoading] = useState(true);
  const [uploading, setUploading] = useState(0);
  const pendingUploads = useRef(0);
  const loadVersion = useRef(0);
  const input = useRef<HTMLInputElement>(null);
  const nativePicker = useStudio(
    (state) => state.health?.native_file_picker ?? false,
  );
  const version = useStudio((state) => state.projectVersions[projectId] ?? 0);
  const load = useCallback(async () => {
    const version = ++loadVersion.current;
    try {
      const result = await api.assets(projectId, workspace);
      if (loadVersion.current === version) setAssets(result.assets);
    } catch (error) {
      toast.error("Couldn't load assets", (error as Error).message);
    } finally {
      if (loadVersion.current === version) setLoading(false);
    }
  }, [projectId, workspace]);
  useEffect(() => {
    void load();
  }, [load, version]);
  const upload = async (files: File[]) => {
    if (!files.length) return;
    pendingUploads.current += files.length;
    setUploading(pendingUploads.current);
    try {
      for (const file of files) {
        try {
          await api.uploadAsset(projectId, file, workspace);
        } catch (error) {
          toast.error(`Couldn't add ${file.name}`, (error as Error).message);
        } finally {
          pendingUploads.current -= 1;
          setUploading(pendingUploads.current);
        }
      }
    } finally {
      await load();
    }
  };
  const remove = async (asset: Asset) => {
    if (
      !(await confirmAction({
        title: `Delete ${asset.path}?`,
        description: "Scenes using this file will need a replacement.",
        confirm: "Delete asset",
        destructive: true,
      }))
    )
      return;
    try {
      await api.deleteAsset(projectId, asset.path, workspace);
      await load();
    } catch (error) {
      toast.error("Couldn't delete asset", (error as Error).message);
    }
  };
  const choose = async () => {
    if (!nativePicker) {
      if (input.current?.showPicker) input.current.showPicker();
      else input.current?.click();
      return;
    }
    pendingUploads.current += 1;
    setUploading(pendingUploads.current);
    try {
      const result = await api.chooseAssets(projectId, "documents", workspace);
      for (const error of result.errors)
        toast.error(`Couldn't add ${error.file}`, error.message);
    } catch (error) {
      toast.error("Couldn't choose files", (error as Error).message);
    } finally {
      pendingUploads.current -= 1;
      setUploading(pendingUploads.current);
      await load();
    }
  };
  const drop = useFileDrop((files) => void upload(files));
  return (
    <div
      className={cn(
        "flex min-h-0 flex-1 flex-col transition-shadow",
        drop.dragging && "shadow-[inset_0_0_0_2px_var(--primary)]",
      )}
      {...drop.props}
    >
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-3">
        <span className="flex-1 text-xs text-muted-foreground">
          {assets.length === 1 ? "1 asset" : `${assets.length} assets`}
        </span>
        <Button
          size="xs"
          variant="ghost-muted"
          onClick={() =>
            void api
              .openProjectAssets(projectId, workspace)
              .catch((error: Error) =>
                toast.error("Couldn't open folder", error.message),
              )
          }
        >
          <FolderOpenIcon /> Folder
        </Button>
        <Button
          size="xs"
          variant="outline"
          disabled={uploading > 0}
          onClick={() => void choose()}
        >
          {uploading ? <Spinner /> : <UploadIcon />} Add files
        </Button>
        <input
          aria-label="Upload assets"
          ref={input}
          type="file"
          multiple
          hidden
          onChange={(event) => {
            void upload([...(event.target.files ?? [])]);
            event.target.value = "";
          }}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        {loading ? (
          <div className="grid place-items-center py-14">
            <Spinner />
          </div>
        ) : !assets.length ? (
          <div className="py-14 text-center text-sm text-muted-foreground">
            <ImageIcon className="mx-auto mb-3 size-7" />
            Drop files here or ask the agent to create assets.
          </div>
        ) : (
          <div className="space-y-2">
            {assets.map((asset) => (
              <div
                key={asset.path}
                className="flex items-center gap-3 rounded-lg border bg-card p-2"
              >
                {asset.kind === "image" || asset.kind === "vector" ? (
                  <img
                    src={api.assetUrl(projectId, asset.path, workspace)}
                    alt=""
                    className="size-12 shrink-0 rounded bg-muted object-contain"
                    loading="lazy"
                  />
                ) : (
                  <span className="grid size-12 shrink-0 place-items-center rounded bg-muted text-xs">
                    {asset.kind}
                  </span>
                )}
                <div className="min-w-0 flex-1">
                  <a
                    href={api.assetUrl(projectId, asset.path, workspace)}
                    target="_blank"
                    rel="noreferrer"
                    className="block truncate text-sm hover:underline"
                    title={asset.path}
                  >
                    {asset.path}
                  </a>
                  <span className="text-xs text-muted-foreground">
                    {formatBytes(asset.size)}
                  </span>
                </div>
                <Button
                  size="icon-xs"
                  variant="ghost-muted"
                  aria-label={`Copy path for ${asset.path}`}
                  onClick={() =>
                    void navigator.clipboard
                      .writeText(`assets/${asset.path}`)
                      .then(() => toast.success("Asset path copied"))
                      .catch(() => toast.error("Couldn't copy path"))
                  }
                >
                  <CopyIcon />
                </Button>
                <Button
                  size="icon-xs"
                  variant="ghost-muted"
                  aria-label={`Delete ${asset.path}`}
                  onClick={() => void remove(asset)}
                >
                  <Trash2Icon />
                </Button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
