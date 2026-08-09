import { createApp } from "vue";
import App from "./App.vue";

window.addEventListener("error", (e) => {
  document.body.style.background = "#111";
  document.body.style.color = "#ff5252";
  document.body.style.padding = "20px";
  document.body.style.fontFamily = "monospace";
  document.body.style.whiteSpace = "pre-wrap";
  document.body.innerHTML = "JS 错误: " + (e.message || e.error?.stack || JSON.stringify(e));
});

window.addEventListener("unhandledrejection", (e) => {
  console.error("UNHANDLED REJECTION:", e.reason);
});

const app = createApp(App);
app.config.errorHandler = (err, instance, info) => {
  console.error("Vue error:", err, info);
};
app.mount("#app");
