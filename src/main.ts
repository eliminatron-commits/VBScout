import { mount } from 'svelte';
import App from './App.svelte';
import { product } from './lib/product';
import './app.css';

document.title = product.name;

const target = document.getElementById('app');
if (!target) throw new Error('#app element missing');

export default mount(App, { target });
