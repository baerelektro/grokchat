import { registerRootComponent } from 'expo';
import App from './App';

// Коротко про этот файл:
// 1) Это стандартная точка входа Expo-приложения.
// 2) Регистрирует корневой React-компонент.
// 3) Вся логика интерфейса живёт в App.js, здесь только bootstrap.
registerRootComponent(App);
