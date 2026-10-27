import { useEffect, useState } from "react";
import type { Entry } from "@/types";
import { previewInfo, previewThumbnail, type PreviewInfo } from "@/lib/ipc";
import { formatDate, formatSize, typeLabel } from "@/lib/format";
import { Icon } from "./Icons";

/**
 * The preview pane, on the right, showing the focused file.
 *
 * The picture is a thumbnail made by the main process and handed over as bytes,
 * not a path and not the original file. Decoding a large photograph at full
 * size to show it a few hundred pixels wide is how a file manager ends up using
 * hundreds of megabytes to display one image.
 */
export function PreviewPane({ entry }: { entry: Entry | null }) {
  const [info, setInfo] = useState<PreviewInfo | null>(null);
  const [picture, setPicture] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let disposed = false;
    let url: string | null = null;

    setInfo(null);
    setPicture(null);
    setFailed(false);
    if (!entry) return;

    void previewInfo(entry.path)
      .then((next) => {
        if (disposed) return;
        setInfo(next);
        if (!next.hasThumbnail) return;
        return previewThumbnail(entry.path).then((blob) => {
          if (disposed) return;
          if (!blob) {
            // A tool that produced nothing is a failure, not a pending state.
            setFailed(true);
            return;
          }
          url = blob;
          setPicture(blob);
        });
      })
      .catch(() => {
        if (!disposed) setFailed(true);
      });

    return () => {
      disposed = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [entry]);

  const modified = info?.modifiedMs ? formatDate(info.modifiedMs) : null;

  return (
    <aside className="preview" aria-label="Preview">
      <div className="preview-stage">
        {picture ? (
          <img className="preview-image" src={picture} alt={entry?.name ?? ""} />
        ) : (
          <span className="preview-placeholder" data-failed={failed ? "true" : "false"}>
            <Icon
              name={
                entry?.isDir
                  ? "folder"
                  : info?.kind === "video"
                    ? "video"
                    : info?.kind === "audio"
                      ? "audio"
                      : info?.kind === "pdf" || info?.kind === "text"
                        ? "document"
                        : "document"
              }
              size={48}
              strokeWidth={0.9}
            />
            {entry && info?.hasThumbnail && !failed ? (
              <span className="preview-note">Making a preview…</span>
            ) : null}
            {failed ? <span className="preview-note">No preview available</span> : null}
          </span>
        )}
      </div>

      {entry ? (
        <>
          <div className="preview-name" title={entry.path}>
            {entry.name}
          </div>
          {info?.excerpt ? (
            <pre className="preview-text">{info.excerpt}</pre>
          ) : null}
          <dl className="preview-meta">
            <dt>Type</dt>
            <dd>{typeLabel(entry)}</dd>
            <dt>Size</dt>
            <dd>{entry.isDir ? "—" : formatSize(entry.size, false)}</dd>
            {modified ? (
              <>
                <dt>Modified</dt>
                <dd>{modified}</dd>
              </>
            ) : null}
          </dl>
        </>
      ) : (
        <p className="preview-empty">Select a file to see it here.</p>
      )}
    </aside>
  );
}
