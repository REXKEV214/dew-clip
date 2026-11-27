import React from "react";
import ReactDOM from "react-dom/client";
import Capsule from "./Capsule";
import { applyEffectiveTheme, mirroredAppearance, resolveInitialTheme } from "../shared/lib/appearance";
import "./capsule.css";

applyEffectiveTheme(resolveInitialTheme(mirroredAppearance()));

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Capsule />
  </React.StrictMode>
);
