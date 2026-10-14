import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import "./index.css";
import { router } from "./router";
import { useProjectDoc } from "./state/project";
import { useStudio } from "./state/studio";

Object.assign(window, {
  studioLifecycle: {
    flush: async () => {
      const saved = await Promise.all([
        useProjectDoc.getState().flush(),
        useStudio.getState().flushPrefs(),
      ]);
      return saved.every(Boolean);
    },
  },
});

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>,
);
