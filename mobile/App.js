import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  SafeAreaView,
  View,
  Text,
  TextInput,
  Pressable,
  FlatList,
  StyleSheet,
  KeyboardAvoidingView,
  Platform,
} from 'react-native';
import { StatusBar } from 'expo-status-bar';

function nowTime() {
  const date = new Date();
  return date.toLocaleTimeString();
}

export default function App() {
  const [host, setHost] = useState('192.168.1.10');
  const [port, setPort] = useState('8080');
  const [input, setInput] = useState('');
  const [connected, setConnected] = useState(false);
  const [messages, setMessages] = useState([]);

  const socketRef = useRef(null);

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
    addChat('me', text);
    setInput('');
  };

  useEffect(() => {
    return () => disconnect();
  }, []);

  return (
    <SafeAreaView style={styles.safeArea}>
      <StatusBar style="light" />
      <KeyboardAvoidingView
        behavior={Platform.OS === 'ios' ? 'padding' : undefined}
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
        </View>

        <Text style={styles.status}>
          Статус: {connected ? '🟢 online' : '🔴 offline'}
        </Text>

        <FlatList
          style={styles.list}
          data={messages}
          keyExtractor={(item) => item.id}
          renderItem={({ item }) => (
            <View style={item.kind === 'system' ? styles.systemBubble : styles.chatBubble}>
              {item.kind === 'chat' && <Text style={styles.from}>{item.from}</Text>}
              <Text style={styles.msgText}>{item.text}</Text>
              <Text style={styles.time}>{item.time}</Text>
            </View>
          )}
        />

        <View style={styles.row}>
          <TextInput
            value={input}
            onChangeText={setInput}
            placeholder="Сообщение"
            placeholderTextColor="#8a90a3"
            style={[styles.input, styles.msgInput]}
            onSubmitEditing={sendMessage}
          />
          <Pressable style={styles.sendBtn} onPress={sendMessage}>
            <Text style={styles.btnText}>Отправить</Text>
          </Pressable>
        </View>
      </KeyboardAvoidingView>
    </SafeAreaView>
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
  list: {
    flex: 1,
    backgroundColor: '#11172c',
    borderRadius: 12,
    padding: 8,
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
  sendBtn: {
    backgroundColor: '#2f6bff',
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderRadius: 10,
  },
});
