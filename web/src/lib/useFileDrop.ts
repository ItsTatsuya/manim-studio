import { useRef, useState, type DragEvent } from "react";

/**
 * Drop-zone handlers that also report whether a file is being dragged over,
 * so the zone can highlight itself. Nested children fire enter/leave pairs,
 * so a depth counter keeps the highlight steady.
 */
export function useFileDrop(onFiles: (files: File[]) => void) {
  const [dragging, setDragging] = useState(false);
  const depth = useRef(0);
  const carriesFiles = (event: DragEvent) =>
    [...event.dataTransfer.types].includes("Files");
  return {
    dragging,
    props: {
      onDragEnter(event: DragEvent) {
        if (!carriesFiles(event)) return;
        depth.current += 1;
        setDragging(true);
      },
      onDragLeave(event: DragEvent) {
        if (!carriesFiles(event)) return;
        depth.current = Math.max(0, depth.current - 1);
        if (!depth.current) setDragging(false);
      },
      onDragOver(event: DragEvent) {
        event.preventDefault();
      },
      onDrop(event: DragEvent) {
        event.preventDefault();
        depth.current = 0;
        setDragging(false);
        onFiles([...event.dataTransfer.files]);
      },
    },
  };
}
