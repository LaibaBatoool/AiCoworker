import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./theme-black.css"; // black theme (remove this line for the violet one)

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
