import 'dart:ui';

import 'package:flutter/widgets.dart';

Color nveColorFromRrggbbaa(int rgba) =>
    Color.fromARGB(rgba & 0xFF, (rgba >> 24) & 0xFF, (rgba >> 16) & 0xFF, (rgba >> 8) & 0xFF);

enum NveBlurMaterial {
  ultraThin(0, 8.0),
  thin(1, 14.0),
  regular(2, 20.0),
  thick(3, 30.0),
  chrome(4, 24.0),
  systemMaterial(5, 20.0);

  const NveBlurMaterial(this.wireCode, this.baseSigma);
  final int wireCode;
  final double baseSigma;

  static NveBlurMaterial fromCode(int code) =>
      (code >= 0 && code < values.length) ? values[code] : NveBlurMaterial.regular;
}

class NveBlurSurfaceProps {
  const NveBlurSurfaceProps({
    this.material = NveBlurMaterial.regular,
    this.intensity = 1.0,
    this.tintRgba,
  });

  final NveBlurMaterial material;
  final double intensity;
  final int? tintRgba;

  double get clampedIntensity => intensity.isFinite ? intensity.clamp(0.0, 1.0) : 1.0;
  double get sigma => material.baseSigma * clampedIntensity;
}

class NveFlutterBlurSurfaceState extends ChangeNotifier {
  NveBlurSurfaceProps props = const NveBlurSurfaceProps();
  bool unmounted = false;

  void apply(NveBlurSurfaceProps next) {
    props = next;
    notifyListeners();
  }

  void markUnmounted() {
    unmounted = true;
    dispose();
  }
}

class NveFlutterBlurSurfaceWidget extends StatelessWidget {
  const NveFlutterBlurSurfaceWidget({super.key, required this.state, this.child});
  final NveFlutterBlurSurfaceState state;
  final Widget? child;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
        listenable: state,
        builder: (_, __) {
          final double s = state.props.sigma;
          final int? tint = state.props.tintRgba;
          return ClipRect(
            child: BackdropFilter(
              filter: ImageFilter.blur(sigmaX: s, sigmaY: s),
              child: tint != null
                  ? ColoredBox(
                      color: nveColorFromRrggbbaa(tint),
                      child: child ?? const SizedBox.expand(),
                    )
                  : (child ?? const SizedBox.expand()),
            ),
          );
        },
      );
}
