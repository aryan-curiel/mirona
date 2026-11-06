import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/inter/700.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "./styles.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { SettingsWindow } from "./views/Settings";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {getCurrentWindow().label === "settings" ? <SettingsWindow /> : <App />}
  </React.StrictMode>,
);
