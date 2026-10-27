import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/chrome.css";
import "./styles/content.css";

const root = document.getElementById("root");
if (!root) throw new Error("Lyre: #root is missing from index.html");

createRoot(root).render(<App />);
