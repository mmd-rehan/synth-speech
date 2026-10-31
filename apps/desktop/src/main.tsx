import React from "react";
import ReactDOM from "react-dom/client";

import App from "@/App";
import { initTheme } from "@/lib/theme";
import "@/lib/i18n";
import "@/index.css";

initTheme();

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("root element not found");

ReactDOM.createRoot(rootEl).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
