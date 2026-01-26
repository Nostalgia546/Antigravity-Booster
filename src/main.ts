import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import "./style.css";

const app = createApp(App);
app.use(createPinia());
app.mount("#app");

// Disable Right-Click Menu for App-like feel
document.addEventListener('contextmenu', event => event.preventDefault());
