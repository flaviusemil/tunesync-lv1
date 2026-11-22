// window.h - the settings window (docs/settings-onepage.html): its own thread and
// message loop, never LV1's UI thread. Drawn with Direct2D / DirectWrite from the
// portable view model in ui/.
#pragma once

namespace trw {

class App;

class Window {
  public:
    // Starts the window thread (the window itself is created hidden). nullptr on failure.
    static Window* start(App* app);
    void show();                 // any thread: posts; brings it to the front
    void stop(unsigned waitMs);  // closes the window and ends its thread (bounded)
  private:
    Window() = default;
    struct Impl;
    Impl* impl_ = nullptr;
};

}  // namespace trw
