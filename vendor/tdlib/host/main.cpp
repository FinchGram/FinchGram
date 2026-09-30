// finchgram-tdlib: TDLib's JSON interface over standard input and output.
//
// FinchGram talks to Telegram only through this program, which sits next to its executable the way
// ffmpeg does for Coova Studio (docs/architecture.md). The messages are TDLib's own JSON objects,
// one per line in both directions, UTF-8:
//
//   standard input    requests, handed to td_send as they are
//   standard output   everything td_receive returns: answers, which carry the "@extra" of their
//                     request, and updates
//   standard error    TDLib's log
//
// One program runs one TDLib client, that is one account. When standard input closes, because the
// app quit or crashed, TDLib is asked to close, which writes its database out; the program ends
// once TDLib reports authorizationStateClosed, as it also does after a log out.
//
//   finchgram-tdlib             run
//   finchgram-tdlib --version   print TDLib's version and commit, and exit

#include <td/telegram/td_json_client.h>

#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <thread>

namespace {

// Errors only. The app asks for more with setLogVerbosityLevel when it wants it.
const char *const kQuietLog = R"({"@type":"setLogVerbosityLevel","new_verbosity_level":1})";

// The value of a string option, read synchronously ("version" and "commit_hash" allow that).
std::string option_value(const char *name) {
  const std::string request = std::string(R"({"@type":"getOption","name":")") + name + R"("})";
  const char *answer = td_execute(request.c_str());
  const std::string text = answer != nullptr ? answer : "";
  const std::string key = R"("value":")";
  const size_t start = text.find(key);
  if (start == std::string::npos) {
    return "unknown";
  }
  const size_t end = text.find('"', start + key.size());
  return text.substr(start + key.size(), end == std::string::npos ? std::string::npos : end - start - key.size());
}

// TDLib writes every object on one line, so a line is one message.
void write_message(const char *message) {
  std::fputs(message, stdout);
  std::fputc('\n', stdout);
  std::fflush(stdout);
}

bool is_closed(const char *message) {
  return std::strstr(message, R"("@type":"updateAuthorizationState")") != nullptr &&
         std::strstr(message, R"("@type":"authorizationStateClosed")") != nullptr;
}

}  // namespace

int main(int argc, char *argv[]) {
  if (argc == 2 && std::strcmp(argv[1], "--version") == 0) {
    std::printf("%s %s\n", option_value("version").c_str(), option_value("commit_hash").c_str());
    return 0;
  }
  if (argc != 1) {
    std::fprintf(stderr, "usage: finchgram-tdlib [--version]\n");
    return 2;
  }

  // Once the app is gone, writing to standard output must fail instead of ending this program
  // before TDLib has closed its database.
  std::signal(SIGPIPE, SIG_IGN);
  td_execute(kQuietLog);

  const int client_id = td_create_client_id();
  std::thread input([client_id] {
    std::string line;
    while (std::getline(std::cin, line)) {
      if (!line.empty()) {
        td_send(client_id, line.c_str());
      }
    }
    td_send(client_id, R"({"@type":"close"})");
  });
  input.detach();

  while (true) {
    const char *message = td_receive(60.0);
    if (message == nullptr) {
      continue;
    }
    write_message(message);
    if (is_closed(message)) {
      // Everything is on disk once TDLib is closed. Leave at once: the input thread may still be
      // waiting for a line, and running static destructors under it would not be safe.
      std::_Exit(0);
    }
  }
}
