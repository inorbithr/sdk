import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app.tsx";
import { ContextMenuHost } from "./frame/context-menu.tsx";
import { followSystemDark } from "./lib/theme.ts";
import "./styles.css";

followSystemDark();

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
      <ContextMenuHost />
    </StrictMode>,
  );
}
