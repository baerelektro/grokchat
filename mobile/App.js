import React, { useEffect, useMemo, useRef, useState } from 'react';
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

function nowTime() {
  const date = new Date();
  return date.toLocaleTimeString();
}

function detectDefaultHost() {
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
  const [host, setHost] = useState(detectDefaultHost);
  const [port, setPort] = useState('8080');
  const [input, setInput] = useState('');
  const [username, setUsername] = useState('me');
  const [draftUsername, setDraftUsername] = useState('me');
  const [screen, setScreen] = useState('chat');
  const [connected, setConnected] = useState(false);
  const [messages, setMessages] = useState([]);

  const socketRef = useRef(null);
  const listRef = useRef(null);

  const wsUrl = useMemo(() => `ws://${host.trim()}:${port.trim()}/ws`, [host, port]);

  const addSystem = (text) => {
    setMessages((prev) => [
      ...prev,
      { id: `${Date.now()}-${Math.random()}`, kind: 'system', text, time: nowTime() },
    ]);
  };

  const addChat = (from, text) => {
    setMessages((prev) => [
      ...prev,
      { id: `${Date.now()}-${Math.random()}`, kind: 'chat', from, text, time: nowTime() },
    ]);
  };

  const disconnect = () => {
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
    };

    ws.onmessage = (event) => {
      try {
        const parsed = JSON.parse(event.data);
        if (parsed.type === 'chat_message') {
          addChat(parsed.from || 'peer', parsed.text || '');
        } else if (parsed.type === 'system') {
          addSystem(parsed.message || 'system');
        } else {
          addSystem(`Неизвестное событие: ${event.data}`);
        }
      } catch {
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
    addChat(username || 'me', text);
    setInput('');
  };

  const openSettings = () => {
    setDraftUsername(username || 'me');
    setScreen('settings');
  };

  const saveSettings = () => {
    const value = draftUsername.trim();
    setUsername(value || 'me');
    setScreen('chat');
  };

  const scrollToBottom = (animated = true) => {
    if (!listRef.current) {
      return;
    }

    listRef.current.scrollToEnd({ animated });
  };

  useEffect(() => {
    if (messages.length > 0) {
      requestAnimationFrame(() => scrollToBottom(true));
    }
  }, [messages.length]);

  useEffect(() => {
    const showEvent = Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow';
    const sub = Keyboard.addListener(showEvent, () => {
      requestAnimationFrame(() => scrollToBottom(true));
    });

    return () => sub.remove();
  }, []);

  useEffect(() => {
    return () => disconnect();
  }, []);

  return (
    <SafeAreaProvider>
      <SafeAreaView style={styles.safeArea}>
        <StatusBar style="light" />
        {screen === 'settings' ? (
          <View style={styles.container}>
            <Text style={styles.title}>Настройки</Text>
            <Text style={styles.label}>Юзернейм</Text>
            <TextInput
              value={draftUsername}
              onChangeText={setDraftUsername}
              placeholder="Введите юзернейм"
              placeholderTextColor="#8a90a3"
              style={styles.input}
              autoCapitalize="none"
            />

            <View style={styles.row}>
              <Pressable style={styles.btn} onPress={saveSettings}>
                <Text style={styles.btnText}>Сохранить</Text>
              </Pressable>
              <Pressable style={[styles.btn, styles.btnSecondary]} onPress={() => setScreen('chat')}>
                <Text style={styles.btnText}>Назад</Text>
              </Pressable>
            </View>
          </View>
        ) : (
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
          <Pressable style={styles.btn} onPress={connect}>
            <Text style={styles.btnText}>Подключить</Text>
          </Pressable>
          <Pressable style={[styles.btn, styles.btnSecondary]} onPress={disconnect}>
            <Text style={styles.btnText}>Отключить</Text>
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
          keyExtractor={(item) => item.id}
          onContentSizeChange={() => scrollToBottom(false)}
          renderItem={({ item }) => (
            <View style={item.kind === 'system' ? styles.systemBubble : styles.chatBubble}>
              {item.kind === 'chat' && <Text style={styles.from}>{item.from}</Text>}
              <Text style={styles.msgText}>{item.text}</Text>
              <Text style={styles.time}>{item.time}</Text>
            </View>
          )}
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
  btnText: {
    color: '#fff',
    fontWeight: '600',
  },
  status: {
    color: '#d7dcf0',
    fontSize: 13,
  },
  label: {
    color: '#d7dcf0',
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
