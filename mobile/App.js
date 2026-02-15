import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Constants from 'expo-constants';
import {
  View,
  Text,
  TextInput,
  Pressable,
  FlatList,
  StyleSheet,
  KeyboardAvoidingView,
  Keyboard,
  Platform,
} from 'react-native';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider, SafeAreaView } from 'react-native-safe-area-context';

// Коротко про этот файл:
// 1) Реализует весь мобильный UI (чат + настройки).
// 2) Держит WebSocket-соединение с backend gateway.
// 3) Управляет ником: локальная валидация + серверная проверка уникальности.

// Ограничения на ник должны совпадать с backend,
// чтобы пользователь сразу видел корректные ошибки на клиенте.
const USERNAME_MIN_LEN = 3;
const USERNAME_MAX_LEN = 24;
const MAX_MESSAGES = 300;

const MessageItem = React.memo(function MessageItem({ item }) {
  return (
    <View style={item.kind === 'system' ? styles.systemBubble : styles.chatBubble}>
      {item.kind === 'chat' && <Text style={styles.from}>{item.from}</Text>}
      <Text style={styles.msgText}>{item.text}</Text>
      <Text style={styles.time}>{item.time}</Text>
    </View>
  );
});

// Форматируем текущее время для меток сообщений в ленте.
function nowTime() {
  const date = new Date();
  return date.toLocaleTimeString();
}

function detectDefaultHost() {
  // Пробуем достать IP хоста из разных Expo-полей манифеста,
  // потому что структура может отличаться между версиями/режимами запуска.
  const candidates = [
    Constants.expoConfig?.hostUri,
    Constants.manifest2?.extra?.expoClient?.hostUri,
    Constants.manifest?.debuggerHost,
  ];

  for (const rawValue of candidates) {
    if (!rawValue || typeof rawValue !== 'string') {
      continue;
    }

    const base = rawValue.split('/')[0].split('?')[0];
    const host = base.includes(':') ? base.split(':')[0] : base;
    if (host && host !== '0.0.0.0') {
      return host;
    }
  }

  return '192.168.1.10';
}

export default function App() {
  // Единая нормализация ника во всём UI.
  function normalizeUsername(value) {
    return value.trim().toLowerCase();
  }

  // Локальная проверка формата ника (без сети).
  function validateUsernameLocal(value) {
    if (!value) {
      return 'Ник не должен быть пустым';
    }
    if (value.length < USERNAME_MIN_LEN) {
      return `Минимум ${USERNAME_MIN_LEN} символа`;
    }
    if (value.length > USERNAME_MAX_LEN) {
      return `Максимум ${USERNAME_MAX_LEN} символа`;
    }
    if (!/^[a-z0-9_.-]+$/i.test(value)) {
      return "Разрешены только a-z, 0-9, '.', '_' и '-'";
    }
    return null;
  }

  const [host, setHost] = useState(detectDefaultHost);
  const [port, setPort] = useState('8080');
  const [input, setInput] = useState('');
  const [username, setUsername] = useState('me');
  const [draftUsername, setDraftUsername] = useState('me');
  const [usernameStatus, setUsernameStatus] = useState(null);
  const [showUsernameTooltip, setShowUsernameTooltip] = useState(false);
  const [isSavingUsername, setIsSavingUsername] = useState(false);
  const [screen, setScreen] = useState('chat');
  const [connected, setConnected] = useState(false);
  const [messages, setMessages] = useState([]);

  // Храним сокет в ref, чтобы не терять ссылку между рендерами.
  const socketRef = useRef(null);
  const listRef = useRef(null);
  // Таймер ожидания ответа на set_username.
  const saveTimeoutRef = useRef(null);
  // Какой ник прямо сейчас пытаемся сохранить.
  const pendingSaveUsernameRef = useRef(null);
  const screenRef = useRef('chat');
  const draftUsernameRef = useRef('me');

  const wsUrl = useMemo(() => `ws://${host.trim()}:${port.trim()}/ws`, [host, port]);
  const httpBaseUrl = useMemo(() => `http://${host.trim()}:${port.trim()}`, [host, port]);
  const normalizedDraftUsername = useMemo(
    () => normalizeUsername(draftUsername),
    [draftUsername]
  );
  const draftUsernameValidation = useMemo(
    () => validateUsernameLocal(normalizedDraftUsername),
    [normalizedDraftUsername]
  );
  const canSaveUsername = useMemo(
    () =>
      connected &&
      !draftUsernameValidation &&
      !isSavingUsername &&
      normalizedDraftUsername !== normalizeUsername(username),
    [
      connected,
      draftUsernameValidation,
      isSavingUsername,
      normalizedDraftUsername,
      username,
    ]
  );

  const usernameIndicator = useMemo(() => {
    // Индикатор в настройках:
    // • нет статуса, ⏳ проверка/сохранение, ✅ свободно, ❌ занято/ошибка.
    if (!usernameStatus) {
      return { icon: '•', color: '#8a90a3' };
    }

    const message = (usernameStatus.message || '').toLowerCase();
    if (message.includes('проверяем')) {
      return { icon: '⏳', color: '#ffd166' };
    }

    if (usernameStatus.available) {
      return { icon: '✅', color: '#7ddc84' };
    }

    return { icon: '❌', color: '#ff8d8d' };
  }, [usernameStatus]);

  const appendMessage = useCallback((message) => {
    setMessages((prev) => {
      const next = [...prev, message];
      if (next.length > MAX_MESSAGES) {
        return next.slice(next.length - MAX_MESSAGES);
      }
      return next;
    });
  }, []);

  const addSystem = useCallback((text) => {
    // Системные сообщения отображаем в той же ленте для прозрачной диагностики.
    appendMessage({
      id: `${Date.now()}-${Math.random()}`,
      kind: 'system',
      text,
      time: nowTime(),
    });
  }, [appendMessage]);

  const addChat = useCallback((from, text) => {
    // Вставляем сообщение в конец списка.
    appendMessage({
      id: `${Date.now()}-${Math.random()}`,
      kind: 'chat',
      from,
      text,
      time: nowTime(),
    });
  }, [appendMessage]);

  const keyExtractor = useCallback((item) => item.id, []);
  const renderItem = useCallback(({ item }) => <MessageItem item={item} />, []);

  const disconnect = () => {
    // Явно закрываем старый сокет перед новым connect.
    if (socketRef.current) {
      socketRef.current.close();
      socketRef.current = null;
    }
    setConnected(false);
  };

  const connect = () => {
    disconnect();
    addSystem(`Подключение к ${wsUrl}`);

    const ws = new WebSocket(wsUrl);
    socketRef.current = ws;

    ws.onopen = () => {
      setConnected(true);
      addSystem('WebSocket подключен');

      const normalizedCurrentUsername = normalizeUsername(username);
      if (normalizedCurrentUsername) {
        ws.send(
          JSON.stringify({
            type: 'set_username',
            username: normalizedCurrentUsername,
          })
        );
      }
    };

    ws.onmessage = (event) => {
      try {
        const parsed = JSON.parse(event.data);
        if (parsed.type === 'chat_message') {
          addChat(parsed.from || 'peer', parsed.text || '');
        } else if (parsed.type === 'username_observed') {
          // Служебное событие от backend: в мобильном UI просто игнорируем.
          return;
        } else if (parsed.type === 'username_status') {
          // Фильтруем "чужие" status-события, чтобы не ломать локальный save-flow.
          const normalizedIncoming = normalizeUsername(parsed.username || '');
          const pendingSave = pendingSaveUsernameRef.current;
          const isApplied = Boolean(parsed.applied);
          const currentDraft = normalizeUsername(draftUsernameRef.current || '');

          if (!pendingSave && screenRef.current !== 'settings') {
            return;
          }

          if (pendingSave && normalizedIncoming !== pendingSave) {
            return;
          }

          if (!pendingSave && normalizedIncoming !== currentDraft) {
            return;
          }

          if (!pendingSave && isApplied) {
            return;
          }

          setUsernameStatus({
            available: Boolean(parsed.available),
            applied: isApplied,
            username: normalizedIncoming,
            message: parsed.message || '',
          });

          if (saveTimeoutRef.current) {
            clearTimeout(saveTimeoutRef.current);
            saveTimeoutRef.current = null;
          }
          pendingSaveUsernameRef.current = null;
          setIsSavingUsername(false);

          if (parsed.available && parsed.applied) {
            // Сервер подтвердил и применил ник.
            const normalized = normalizeUsername(parsed.username || '');
            if (normalized) {
              setUsername(normalized);
              addSystem(`Ник сохранён: ${normalized}`);
            }
          }
        } else if (parsed.type === 'system') {
          addSystem(parsed.message || 'system');
        } else {
          addSystem(`Неизвестное событие: ${event.data}`);
        }
      } catch {
        // Если это не JSON протокол — показываем как raw.
        addSystem(`Raw: ${String(event.data)}`);
      }
    };

    ws.onerror = () => {
      addSystem('Ошибка WebSocket');
    };

    ws.onclose = () => {
      setConnected(false);
      addSystem('WebSocket отключен');
    };
  };

  const sendMessage = () => {
    const text = input.trim();
    if (!text || !socketRef.current || socketRef.current.readyState !== WebSocket.OPEN) {
      return;
    }

    socketRef.current.send(JSON.stringify({ type: 'send_message', text }));
    // Оптимистично добавляем локально, чтобы UI был отзывчивее.
    addChat(username || 'me', text);
    setInput('');
  };

  const openSettings = () => {
    setDraftUsername(username || 'me');
    setUsernameStatus(null);
    setShowUsernameTooltip(false);
    setIsSavingUsername(false);
    pendingSaveUsernameRef.current = null;
    setScreen('settings');
  };

  const saveSettings = () => {
    const value = normalizedDraftUsername;
    const localError = draftUsernameValidation;
    const currentNormalized = normalizeUsername(username);

    if (isSavingUsername) {
      return;
    }

    if (value === currentNormalized) {
      setUsernameStatus({
        available: true,
        applied: true,
        username: value,
        message: 'Этот ник уже сохранён',
      });
      return;
    }

    if (localError) {
      setUsernameStatus({ available: false, applied: false, username: value, message: localError });
      return;
    }

    if (!socketRef.current || socketRef.current.readyState !== WebSocket.OPEN) {
      setUsernameStatus({
        available: false,
        applied: false,
        username: value,
        message: 'Для проверки уникальности подключись к серверу',
      });
      return;
    }

    socketRef.current.send(JSON.stringify({ type: 'set_username', username: value }));
    // Переходим в состояние ожидания подтверждения от backend.
    pendingSaveUsernameRef.current = value;
    setIsSavingUsername(true);
    setUsernameStatus({
      available: true,
      applied: false,
      username: value,
      message: 'Сохраняем ник...',
    });

    if (saveTimeoutRef.current) {
      clearTimeout(saveTimeoutRef.current);
    }
    saveTimeoutRef.current = setTimeout(() => {
      // Таймаут, если backend не прислал applied-ответ.
      setUsernameStatus((prev) => {
        if (!prev?.applied) {
          return {
            available: false,
            applied: false,
            username: value,
            message: 'Сервер не ответил на сохранение ника',
          };
        }
        return prev;
      });
      setIsSavingUsername(false);
      pendingSaveUsernameRef.current = null;
      saveTimeoutRef.current = null;
    }, 2500);
  };

  const scrollToBottom = (animated = true) => {
    if (!listRef.current) {
      return;
    }

    listRef.current.scrollToEnd({ animated });
  };

  useEffect(() => {
    // Автопрокрутка вниз на каждое новое сообщение.
    if (messages.length > 0) {
      requestAnimationFrame(() => scrollToBottom(true));
    }
  }, [messages.length]);

  useEffect(() => {
    // При открытии клавиатуры прокручиваем список к низу,
    // чтобы поле ввода и последние сообщения были видны.
    const showEvent = Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow';
    const sub = Keyboard.addListener(showEvent, () => {
      requestAnimationFrame(() => scrollToBottom(true));
    });

    return () => sub.remove();
  }, []);

  useEffect(() => {
    // Cleanup при размонтировании экрана.
    return () => {
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }
      disconnect();
    };
  }, []);

  useEffect(() => {
    screenRef.current = screen;
  }, [screen]);

  useEffect(() => {
    draftUsernameRef.current = draftUsername;
  }, [draftUsername]);

  useEffect(() => {
    if (screen !== 'settings') {
      return;
    }

    if (isSavingUsername || pendingSaveUsernameRef.current) {
      return;
    }

    const normalized = normalizeUsername(draftUsername);
    const currentNormalized = normalizeUsername(username);

    if (normalized === currentNormalized) {
      setUsernameStatus((prev) => {
        if (
          prev &&
          prev.available === true &&
          prev.applied === true &&
          prev.username === normalized &&
          prev.message === 'Это текущий ник'
        ) {
          return prev;
        }

        return {
          available: true,
          applied: true,
          username: normalized,
          message: 'Это текущий ник',
        };
      });
      return;
    }

    const localError = validateUsernameLocal(normalized);

    if (localError) {
      setUsernameStatus({
        available: false,
        applied: false,
        username: normalized,
        message: localError,
      });
      return;
    }

    setUsernameStatus({
      available: false,
      applied: false,
      username: normalized,
      message: 'Проверяем доступность ника...',
    });

    const controller = new AbortController();

    const timer = setTimeout(() => {
      // Debounce HTTP-проверки, чтобы не спамить сервер на каждый символ.
      fetch(
        `${httpBaseUrl}/check-username/${encodeURIComponent(normalized)}?current=${encodeURIComponent(
          normalizeUsername(username)
        )}`,
        { signal: controller.signal }
      )
        .then((response) => response.json())
        .then((payload) => {
          if (screenRef.current !== 'settings' || isSavingUsername || pendingSaveUsernameRef.current) {
            return;
          }

          const normalizedDraft = normalizeUsername(draftUsername);
          const normalizedIncoming = normalizeUsername(payload.username || '');
          // Защита от гонки: игнорируем ответ не для текущего draft-значения.
          if (normalizedDraft && normalizedIncoming && normalizedDraft !== normalizedIncoming) {
            return;
          }

          setUsernameStatus({
            available: Boolean(payload.available),
            applied: false,
            username: normalizedIncoming,
            message: payload.message || 'Не удалось проверить ник',
          });
        })
        .catch((error) => {
          if (error?.name === 'AbortError') {
            return;
          }

          setUsernameStatus({
            available: false,
            applied: false,
            username: normalized,
            message: 'Ошибка проверки ника (HTTP)',
          });
        });
    }, 300);

    return () => {
      clearTimeout(timer);
      controller.abort();
    };
  }, [draftUsername, screen, host, port, username, isSavingUsername]);

  return (
    <SafeAreaProvider>
      <SafeAreaView style={styles.safeArea}>
        <StatusBar style="light" />
        {screen === 'settings' ? (
          // Экран настроек: редактирование и сохранение ника.
          <View style={styles.container}>
            <Text style={styles.title}>Настройки</Text>
            <Text style={styles.label}>Юзернейм</Text>
            <TextInput
              value={draftUsername}
              onChangeText={(value) => {
                setDraftUsername(value);
              }}
              placeholder="Введите юзернейм"
              placeholderTextColor="#8a90a3"
              style={styles.input}
              autoCapitalize="none"
            />

            <View style={styles.usernameIndicatorRow}>
              <Pressable
                onPress={() => setShowUsernameTooltip((prev) => !prev)}
                style={styles.usernameIndicatorBtn}
              >
                <Text style={[styles.usernameIndicatorIcon, { color: usernameIndicator.color }]}>
                  {usernameIndicator.icon}
                </Text>
              </Pressable>
              <Text style={styles.usernameIndicatorHint}>Нажми на значок</Text>
            </View>

            {showUsernameTooltip && usernameStatus && (
              <View style={styles.usernameTooltip}>
                <Text
                  style={
                    usernameStatus.available ? styles.usernameOk : styles.usernameError
                  }
                >
                  {usernameStatus.message}
                </Text>
              </View>
            )}

            <View style={styles.row}>
              <Pressable
                style={[styles.btn, !canSaveUsername && styles.btnDisabled]}
                onPress={saveSettings}
                disabled={!canSaveUsername}
              >
                <Text style={styles.btnText}>Сохранить</Text>
              </Pressable>
              <Pressable style={[styles.btn, styles.btnSecondary]} onPress={() => setScreen('chat')}>
                <Text style={styles.btnText}>Назад</Text>
              </Pressable>
            </View>
          </View>
        ) : (
        // Основной экран чата.
        <KeyboardAvoidingView
          behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
          keyboardVerticalOffset={Platform.OS === 'ios' ? 0 : 20}
          style={styles.container}
        >
          <Text style={styles.title}>GrokChat Mobile</Text>

        <View style={styles.row}>
          <TextInput
            value={host}
            onChangeText={setHost}
            placeholder="IP сервера"
            placeholderTextColor="#8a90a3"
            style={[styles.input, styles.hostInput]}
            autoCapitalize="none"
          />
          <TextInput
            value={port}
            onChangeText={setPort}
            placeholder="Порт"
            placeholderTextColor="#8a90a3"
            style={[styles.input, styles.portInput]}
            keyboardType="numeric"
          />
        </View>

        <View style={styles.row}>
          <Pressable
            style={({ pressed }) => [
              styles.btn,
              connected && styles.btnInactive,
              pressed && !connected && styles.btnPressed,
            ]}
            onPress={connect}
            disabled={connected}
          >
            <Text style={[styles.btnText, connected && styles.btnTextInactive]}>Подключить</Text>
          </Pressable>
          <Pressable
            style={({ pressed }) => [
              connected ? styles.btn : styles.btnSecondary,
              !connected && styles.btnInactive,
              pressed && connected && styles.btnPressed,
            ]}
            onPress={disconnect}
            disabled={!connected}
          >
            <Text style={[styles.btnText, !connected && styles.btnTextInactive]}>Отключить</Text>
          </Pressable>
          <Pressable style={[styles.btn, styles.gearBtn]} onPress={openSettings}>
            <Text style={styles.btnText}>⚙️</Text>
          </Pressable>
        </View>

        <Text style={styles.status}>
          Статус: {connected ? '🟢 online' : '🔴 offline'} · Ник: {username}
        </Text>

        <FlatList
          ref={listRef}
          style={styles.list}
          contentContainerStyle={styles.listContent}
          keyboardShouldPersistTaps="handled"
          data={messages}
          keyExtractor={keyExtractor}
          renderItem={renderItem}
          initialNumToRender={20}
          maxToRenderPerBatch={20}
          windowSize={7}
          removeClippedSubviews={true}
          onContentSizeChange={() => scrollToBottom(false)}
        />

        <View style={[styles.row, styles.composerRow]}>
          <TextInput
            value={input}
            onChangeText={setInput}
            placeholder="Сообщение"
            placeholderTextColor="#8a90a3"
            style={[styles.input, styles.msgInput]}
            returnKeyType="send"
            blurOnSubmit={false}
            onSubmitEditing={sendMessage}
          />
          <Pressable style={styles.sendBtn} onPress={sendMessage}>
            <Text style={styles.btnText}>Отправить</Text>
          </Pressable>
        </View>
        </KeyboardAvoidingView>
        )}
      </SafeAreaView>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({
  safeArea: {
    flex: 1,
    backgroundColor: '#0b1020',
  },
  container: {
    flex: 1,
    padding: 14,
    gap: 10,
  },
  title: {
    color: '#fff',
    fontSize: 22,
    fontWeight: '700',
  },
  row: {
    flexDirection: 'row',
    gap: 8,
    alignItems: 'center',
  },
  input: {
    backgroundColor: '#171d34',
    color: '#fff',
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 10,
  },
  hostInput: {
    flex: 1,
  },
  portInput: {
    width: 90,
  },
  btn: {
    backgroundColor: '#2f6bff',
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderRadius: 10,
  },
  btnSecondary: {
    backgroundColor: '#39405d',
  },
  btnPressed: {
    backgroundColor: '#586181',
  },
  btnInactive: {
    backgroundColor: '#586181',
  },
  btnDisabled: {
    backgroundColor: '#586181',
  },
  btnText: {
    color: '#fff',
    fontWeight: '600',
  },
  btnTextInactive: {
    color: '#c5cad8',
  },
  status: {
    color: '#d7dcf0',
    fontSize: 13,
  },
  label: {
    color: '#d7dcf0',
    fontSize: 13,
  },
  usernameIndicatorRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
  },
  usernameIndicatorBtn: {
    width: 26,
    height: 26,
    borderRadius: 13,
    backgroundColor: '#171d34',
    alignItems: 'center',
    justifyContent: 'center',
  },
  usernameIndicatorIcon: {
    fontSize: 16,
  },
  usernameIndicatorHint: {
    color: '#8a90a3',
    fontSize: 12,
  },
  usernameTooltip: {
    backgroundColor: '#171d34',
    borderRadius: 8,
    paddingHorizontal: 10,
    paddingVertical: 8,
  },
  usernameOk: {
    color: '#7ddc84',
    fontSize: 13,
  },
  usernameError: {
    color: '#ff8d8d',
    fontSize: 13,
  },
  list: {
    flex: 1,
    backgroundColor: '#11172c',
    borderRadius: 12,
    padding: 8,
  },
  listContent: {
    paddingBottom: 6,
  },
  systemBubble: {
    backgroundColor: '#232a45',
    borderRadius: 10,
    padding: 10,
    marginBottom: 8,
  },
  chatBubble: {
    backgroundColor: '#1b2240',
    borderRadius: 10,
    padding: 10,
    marginBottom: 8,
  },
  from: {
    color: '#9eb2ff',
    fontSize: 12,
    marginBottom: 4,
    fontWeight: '600',
  },
  msgText: {
    color: '#fff',
  },
  time: {
    color: '#8b93b5',
    marginTop: 4,
    fontSize: 11,
  },
  msgInput: {
    flex: 1,
  },
  composerRow: {
    paddingBottom: 4,
  },
  sendBtn: {
    backgroundColor: '#2f6bff',
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderRadius: 10,
  },
  gearBtn: {
    minWidth: 44,
    alignItems: 'center',
  },
});
