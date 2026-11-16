import android.app.Application
import android.content.Context
import android.graphics.Bitmap
import android.graphics.RectF
import android.util.Size
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import androidx.core.telecom.CallControlScope
import com.noma4i.medialayer.*
import com.noma4i.medialayer.ui.*
import com.noma4i.medialayer.testing.*
import java.io.File
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import okhttp3.OkHttpClient

class MomentVideo(override val url: String, override val cover: String? = null) : MediaSource {
  override val kind = Kind.VIDEO
  override val headers = emptyMap<String, String>()
}
class Avatar(override val url: String, override val blurHash: String? = null) : MediaSource {
  override val kind = Kind.IMAGE
  override val headers = emptyMap<String, String>()
}
class Poster(override val url: String) : MediaSource {
  override val kind = Kind.IMAGE
  override val headers = emptyMap<String, String>()
}
typealias Upload = Poster
class VoiceNote(override val url: String) : MediaSource {
  override val kind = Kind.AUDIO
  override val headers = emptyMap<String, String>()
}
typealias IntroClip = MomentVideo
typealias VoiceClip = MomentVideo
typealias MomentCircle = MomentVideo
class Moment(val videoUrl: String, val coverUrl: String, val circleUrl: String, val posterUrl: String)
class User(val avatarUrl: String, val blurHash: String?)
class Message(val remoteUrl: String, val localPreview: String?, val audioUrl: String)
class Clip(val url: String)
class Recording(val file: File)
val media: MediaLayer get() = error("example")
val source: MediaSourceValue get() = error("example")
val moment: Moment get() = error("example")
val user: User get() = error("example")
val message: Message get() = error("example")
val clip: Clip get() = error("example")
val recording: Recording get() = error("example")
val isActive = false
val isShareSheetOpen = false
val url = ""
val avatarUrl = ""
val introUrl = ""
val hash = ""
val blurHash = ""
val specJson = ""
val file: File get() = error("example")
val clipFile: File get() = error("example")
val cacheDirectory: File get() = error("example")
val cacheDir: File get() = error("example")
val webpBytes = byteArrayOf()
val bitmap: Bitmap get() = error("example")
val item: MediaItem get() = error("example")
val introItem: MediaItem get() = error("example")
val video: MomentVideo get() = error("example")
val newItems: List<MediaItem> = emptyList()
val context: Context get() = error("example")
val applicationContext: Context get() = context
val fetcher: Fetcher get() = error("example")
val settings: SettingsStore get() = error("example")
val okHttpClient: OkHttpClient get() = error("example")
val scope: CoroutineScope get() = error("example")
val audio: AudioControl get() = error("example")
val call: MediaCallAudio get() = error("example")
val callControlScope: CallControlScope get() = error("example")
class Row(val avatarUrl: String, val url: String, val video: MediaItem?, val photos: List<MediaItem>)
class Feed(val rows: List<Row>)
val feed: Feed get() = error("example")
val thread: Feed get() = error("example")
val entry: Row get() = error("example")
val rowIndex = 0
val page = 0
val listState: LazyListState get() = error("example")
var cacheBytes = 0uL
var photo: PreparedMedia? = null
var ring = 0.0
var recordEnabled = false
fun dismiss() {}
fun <T> draw(value: T) {}
fun <T> show(value: T) {}
fun showPlaying(value: Boolean) {}
fun hideSplash() {}
fun showHome() {}
fun startCallAudio() {}
fun stopCallAudio() {}
@Composable fun RetryButton() {}
object Sentry { fun captureMessage(message: String) {} }
object R {
  object raw { const val welcome = 0 }
  object drawable { const val banner = 0 }
}
