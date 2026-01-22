import asyncio
import json
import re
import tempfile
import os
import io
import logging
from typing import Optional, Union, Tuple, List, Dict
from enum import Enum

# External library dependencies (standard PyPI packages)
import httpx
import aiofiles
from PIL import Image

# ==========================================
# Reimplemented Constants & Enums
# ==========================================


class DownloadError(Exception):
    """Base error for download failures."""

    pass


class DownloadErrorAgeRestricted(DownloadError):
    """Raised when content is age restricted."""

    pass


class Mimetypes(Enum):
    AUDIO_AAC = "audio/aac"
    AUDIO_MP3 = "audio/mpeg"
    IMAGE_JPEG = "image/jpeg"
    IMAGE_PNG = "image/png"
    VIDEO_MP4 = "video/mp4"


class DownloadQualities(Enum):
    QUALITY_1080 = "1080p"
    QUALITY_720 = "720p"
    QUALITY_480 = "480p"

    def get_dimension(self) -> int:
        return int(self.value.replace("p", ""))


# ==========================================
# Helpers & Tools
# ==========================================


class Mimetyper:
    @staticmethod
    def get_extension_from_mimetype(mimetype: Union[Mimetypes, str]) -> str:
        if isinstance(mimetype, Mimetypes):
            mimetype = mimetype.value

        mapping = {
            "audio/aac": "aac",
            "audio/mpeg": "mp3",
            "image/jpeg": "jpg",
            "image/png": "png",
            "video/mp4": "mp4",
        }
        return mapping.get(mimetype, "dat")

    @staticmethod
    def get_mimetype_from_filename(url: str) -> Mimetypes:
        lower_url = url.lower()
        if ".mp3" in lower_url:
            return Mimetypes.AUDIO_MP3
        if ".aac" in lower_url:
            return Mimetypes.AUDIO_AAC
        if ".png" in lower_url:
            return Mimetypes.IMAGE_PNG
        if ".mp4" in lower_url:
            return Mimetypes.VIDEO_MP4
        return Mimetypes.IMAGE_JPEG


def verify_file_quality(
    width: int, height: int, quality: Optional[DownloadQualities]
) -> Tuple[bool, bool]:
    """
    Returns (is_acceptable, is_exact_match_or_better)
    """
    if not quality:
        return True, True

    target_dim = quality.get_dimension()
    # Check simple logic: is the smallest dimension at least the target?
    min_dim = min(width, height)

    if min_dim >= target_dim:
        return True, True

    # Allow some leniency (e.g. 10% margin)
    if min_dim >= target_dim * 0.9:
        return True, False

    return False, False


# ==========================================
# Media Processing (FFMPEG & PIL)
# ==========================================


class MediaProcessor:
    FFMPEG_BIN = os.getenv("FFMPEG_PATH", "ffmpeg")
    FFPROBE_BIN = os.getenv("FFPROBE_PATH", "ffprobe")

    @classmethod
    async def get_audio_duration(cls, audio_path: str) -> float:
        """Get duration of audio file in seconds using ffprobe."""
        cmd = [
            cls.FFPROBE_BIN,
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            audio_path,
        ]

        process = await asyncio.create_subprocess_exec(
            *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
        )
        stdout, _ = await process.communicate()
        try:
            return float(stdout.decode().strip())
        except ValueError:
            return 0.0

    @classmethod
    def resize_image(
        cls, image_buffer: bytes, target_width: int
    ) -> Tuple[bytes, Mimetypes]:
        """Resize image buffer using PIL."""
        try:
            with Image.open(io.BytesIO(image_buffer)) as img:
                # Calculate new height to maintain aspect ratio
                w_percent = target_width / float(img.size[0])
                h_size = int((float(img.size[1]) * float(w_percent)))

                img = img.resize((target_width, h_size), Image.Resampling.LANCZOS)

                output = io.BytesIO()
                # Convert to RGB to ensure saving as JPEG/PNG works (handles transparency)
                if img.mode in ("RGBA", "P"):
                    img = img.convert("RGB")

                img.save(output, format="PNG")
                return output.getvalue(), Mimetypes.IMAGE_PNG
        except Exception as e:
            logging.error(f"Error resizing image: {e}")
            # Fallback: return original
            return image_buffer, Mimetypes.IMAGE_JPEG

    @classmethod
    async def create_slideshow(
        cls, audio_bytes: bytes, image_bytes: bytes
    ) -> Tuple[bytes, Mimetypes]:
        """Merges a static image and audio into an MP4 video."""

        with tempfile.NamedTemporaryFile(
            suffix=".aac", delete=False
        ) as f_audio, tempfile.NamedTemporaryFile(
            suffix=".png", delete=False
        ) as f_image:

            f_audio.write(audio_bytes)
            f_audio.close()  # Close so ffmpeg can read it

            f_image.write(image_bytes)
            f_image.close()

            audio_duration = await cls.get_audio_duration(f_audio.name)

            # Output container
            output_path = f_image.name + "_out.mp4"

            # FFMPEG Command: Loop image for duration of audio
            cmd = [
                cls.FFMPEG_BIN,
                "-y",
                "-loop",
                "1",
                "-i",
                f_image.name,
                "-i",
                f_audio.name,
                "-c:v",
                "libx264",
                "-tune",
                "stillimage",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-pix_fmt",
                "yuv420p",
                "-t",
                str(audio_duration),
                "-shortest",
                output_path,
            ]

            process = await asyncio.create_subprocess_exec(
                *cmd, stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.PIPE
            )
            _, stderr = await process.communicate()

            if process.returncode != 0:
                # Cleanup
                os.unlink(f_audio.name)
                os.unlink(f_image.name)
                if os.path.exists(output_path):
                    os.unlink(output_path)
                raise DownloadError(f"FFMPEG failed: {stderr.decode()}")

            # Read result back into memory
            async with aiofiles.open(output_path, "rb") as f:
                video_buffer = await f.read()

            # Cleanup
            try:
                os.unlink(f_audio.name)
                os.unlink(f_image.name)
                os.unlink(output_path)
            except OSError:
                pass

            return video_buffer, Mimetypes.VIDEO_MP4


# ==========================================
# Base Service
# ==========================================


class BaseService:
    @classmethod
    def matches(cls, url: str) -> Optional[re.Match]:
        return re.match(cls._VALID_URL, url)


# ==========================================
# Main TikTok Service
# ==========================================


class TikTokService(BaseService):
    _NAME = "tiktok"
    _VALID_URL = r"https?://(?:(?:vt|vm|m|t|www)\.)?tiktok\.com/(?:(?P<user>[^/]+)/(?:video|photo)/(?P<postId>[^/?#&]+)|i18n/share/video/(?P<sharePostId>[^/?#&]+)|(?P<shortLink>[^/?#&]+)|t/(?P<tShortLink>[^/?#&]+)|v/(?P<vPostId>[^./?#&]+)\.html)"

    _HEADERS = {
        "referer": "https://www.tiktok.com/",
        "user-agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    }
    _SHORT_DOMAIN = "https://vt.tiktok.com/"

    @classmethod
    async def _request(
        cls,
        method: str,
        url: str,
        headers: dict = None,
        cookies: dict = None,
        allow_redirects=True,
    ):
        """Internal HTTP client wrapper using httpx."""
        final_headers = cls._HEADERS.copy()
        if headers:
            final_headers.update(headers)

        async with httpx.AsyncClient(
            headers=final_headers, cookies=cookies, follow_redirects=allow_redirects
        ) as client:
            return await client.request(method, url)

    @classmethod
    def unique_id(cls, media_url: str):
        match = cls.matches(media_url)
        if match:
            return (
                match.group("postId")
                or match.group("sharePostId")
                or match.group("vPostId")
                or match.group("shortLink")
                or match.group("tShortLink")
            )
        return None

    @classmethod
    async def download(
        cls,
        media_url: str,
        audio_priority: Optional[bool] = False,
        max_file_size: Optional[int] = None,
        position: int = 0,
        quality: Optional[DownloadQualities] = None,
        **kwargs,
    ) -> Union[None, str, tuple[str, str, Union[None, str], int, str]]:

        match = cls.matches(media_url)
        if not match:
            return None

        # Note: Logic regarding Proxy.get_proxy(special=True) is omitted
        # as it relies on external infrastructure. Using standard request.

        post_id = (
            match.group("postId")
            or match.group("sharePostId")
            or match.group("vPostId")
        )

        short_link = match.group("shortLink") or match.group("tShortLink")
        if short_link and not post_id:
            post_id = await cls._resolve_short_link(short_link)

        if not post_id:
            raise DownloadError("Unable to extract post id from URL")

        details = await cls._get_video_detail(post_id)
        if not details:
            raise DownloadError("Unable to find any media in this url")
        detail, cookies = details

        audio_url = None
        image_url = None
        video_url = None

        should_resize = False

        # --- LOGIC TO DETERMINE MEDIA TYPE ---
        if detail.get("video") and detail["video"]["playAddr"]:
            video_fmt = cls.find_best_format(
                detail["video"]["bitrateInfo"],
                max_file_size=max_file_size,
                quality=quality,
            )
            video_url = video_fmt["PlayAddr"]["UrlList"][0]
            position = 0

        elif detail.get("imagePost"):
            # Image Slide Show
            if detail.get("music"):
                audio_url = detail["music"]["playUrl"]

            if not audio_priority:
                images = detail["imagePost"]["images"]
                position = min(position, len(images) - 1)
                image_url = images[position]["imageURL"]["urlList"][0]

                if not quality:
                    quality = DownloadQualities.QUALITY_1080

                is_maybe_quality, is_quality = verify_file_quality(
                    images[position]["imageWidth"],
                    images[position]["imageHeight"],
                    quality,
                )
                if not is_maybe_quality:
                    should_resize = True
            else:
                position = 0

        elif detail.get("music"):
            audio_url = detail["music"]["playUrl"]
            position = 0

        # Note: Caching logic (maybe_get_cached_file) removed as it requires external DB/Disk logic.
        # Implement your own caching layer here if needed.

        # --- DOWNLOAD LOGIC ---
        buffer = b""
        mimetype = Mimetypes.VIDEO_MP4  # Default

        if audio_url:
            audio_mimetype = Mimetypes.AUDIO_AAC
            if "mime_type=audio_mpeg" in audio_url:
                audio_mimetype = Mimetypes.AUDIO_MP3

            if image_url:
                # Case: Image Slideshow -> Convert to Video
                # Download both audio and image in parallel
                async with httpx.AsyncClient(
                    headers=cls._HEADERS, cookies=cookies
                ) as client:
                    resp_audio_task = client.get(audio_url)
                    resp_image_task = client.get(image_url)
                    resp_audio, resp_image = await asyncio.gather(
                        resp_audio_task, resp_image_task
                    )

                audio_buffer = resp_audio.content
                image_buffer = resp_image.content

                # Determine image type
                image_mimetype = Mimetyper.get_mimetype_from_filename(image_url)

                # Resize if needed
                if quality and should_resize:
                    dimension = quality.get_dimension()
                    image_buffer, image_mimetype = MediaProcessor.resize_image(
                        image_buffer, dimension
                    )

                # Merge into video using FFMPEG
                buffer, mimetype = await MediaProcessor.create_slideshow(
                    audio_buffer, image_buffer
                )

            else:
                # Case: Just Audio
                resp = await cls._request("get", audio_url, cookies=cookies)
                buffer = resp.content
                mimetype = audio_mimetype

        elif image_url:
            # Case: Just Image
            resp = await cls._request("get", image_url, cookies=cookies)
            buffer = resp.content
            mimetype = Mimetyper.get_mimetype_from_filename(image_url)

        elif video_url:
            # Case: Video
            resp = await cls._request("get", video_url, cookies=cookies)
            buffer = resp.content
            mimetype = Mimetypes.VIDEO_MP4
        else:
            raise DownloadError("Unable to find any media in this post")

        # --- SAVE TO FILE ---
        extension = Mimetyper.get_extension_from_mimetype(mimetype)
        filename = f"services-{cls._NAME}-{post_id}-{position}.{extension}"
        filepath = os.path.join(os.getcwd(), filename)

        async with aiofiles.open(filepath, mode="wb") as f:
            await f.write(buffer)

        return filepath, filename, mimetype.value, len(buffer), media_url

    @classmethod
    async def _resolve_short_link(cls, short_link: str) -> Optional[str]:
        try:
            url = cls._SHORT_DOMAIN + short_link
            # Clean UA to look more like a standard request
            headers = {
                "user-agent": cls._HEADERS["user-agent"].split(" Chrome/1")[0],
            }

            # We handle redirects manually or inspect the location
            response = await cls._request(
                "get", url, headers=headers, allow_redirects=False
            )

            # If redirected, the new URL is in headers
            if response.status_code in (301, 302):
                extracted_url = response.headers.get("location", "")
            else:
                # Sometimes it returns a page with a link
                html = response.text
                if html.startswith('<a href="https://'):
                    extracted_url = html.split('<a href="')[1].split("?")[0]
                else:
                    extracted_url = str(response.url)

            patterns = [
                r"/video/(\d+)",
                r"/photo/(\d+)",
                r"/@[^/]+/video/(\d+)",
                r"/@[^/]+/photo/(\d+)",
            ]
            for pattern in patterns:
                match = re.search(pattern, extracted_url)
                if match:
                    return match.group(1)
        except Exception:
            pass
        return None

    @classmethod
    async def _get_video_detail(cls, post_id: str) -> Optional[tuple[dict, dict]]:
        """Get video detail from TikTok"""
        try:
            url = f"https://www.tiktok.com/@i/video/{post_id}"
            response = await cls._request("get", url)

            if response.status_code != 200:
                return None

            html = response.text
            try:
                # TikTok stores data in a hydration script tag
                json_start = '<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">'
                json_end = "</script>"

                start_idx = html.find(json_start)
                if start_idx == -1:
                    return None

                start_idx += len(json_start)
                end_idx = html.find(json_end, start_idx)
                if end_idx == -1:
                    return None

                json_str = html[start_idx:end_idx]
                data = json.loads(json_str)

                video_detail = data.get("__DEFAULT_SCOPE__", {}).get(
                    "webapp.video-detail"
                )

                if not video_detail:
                    return None

                if video_detail.get("statusMsg"):
                    raise DownloadError("Content is unavailable")

                detail = video_detail.get("itemInfo", {}).get("itemStruct")
                if not detail:
                    return None

                if detail.get("isContentClassified"):
                    raise DownloadErrorAgeRestricted("Age Restricted Content")

                # Convert httpx cookies to dict
                cookies_dict = {k: v for k, v in response.cookies.items()}
                return detail, cookies_dict

            except json.JSONDecodeError:
                return None
        except Exception:
            return None

    @classmethod
    def find_best_format(
        cls,
        formats: list[dict],
        max_file_size: Optional[int] = None,
        quality: Optional[DownloadQualities] = None,
    ) -> dict:
        best_score = 0
        best_format = None

        for fmt in formats:
            play_addr = fmt.get("PlayAddr", {})
            data_size = int(play_addr.get("DataSize", 0))

            if max_file_size and data_size > max_file_size:
                continue

            width = play_addr.get("Width", 0)
            height = play_addr.get("Height", 0)

            # Simple heuristic score calculation
            score = width + height - fmt.get("QualityType", 0)

            if quality:
                is_maybe_quality, is_quality = verify_file_quality(
                    width, height, quality
                )
                if not is_maybe_quality:
                    continue
                if is_quality:
                    score *= 10000

            if best_score < score:
                best_format = fmt
                best_score = score

        if not best_format and max_file_size:
            raise DownloadError(f"File is larger than {max_file_size} bytes")

        if not best_format:
            # Fallback to first available if strict logic failed but we have data
            if formats:
                return formats[0]
            raise DownloadError("Unknown Error while retrieving file formats")

        return best_format


if __name__ == "__main__":
    import argparse
    import sys

    # Set up argument parser
    parser = argparse.ArgumentParser(description="TikTok Downloader Service")
    parser.add_argument("url", help="The TikTok URL to download")
    parser.add_argument(
        "--max-size", type=int, help="Max file size in bytes", default=None
    )
    parser.add_argument(
        "--quality", type=str, help="Quality (1080p, 720p, 480p)", default=None
    )
    parser.add_argument(
        "--audio-only", action="store_true", help="Download audio only if possible"
    )

    args = parser.parse_args()

    # Map string quality argument to Enum
    quality_enum = None
    if args.quality:
        try:
            quality_enum = DownloadQualities(args.quality)
        except ValueError:
            print(
                f"Error: Invalid quality '{args.quality}'. Use 1080p, 720p, or 480p.",
                file=sys.stderr,
            )
            sys.exit(1)

    # Run the async download
    try:
        result = asyncio.run(
            TikTokService.download(
                media_url=args.url,
                max_file_size=args.max_size,
                quality=quality_enum,
                audio_priority=args.audio_only,
            )
        )

        if result:
            # Print the resulting JSON to stdout for Rust to parse
            # Format: filepath, filename, mimetype, size, url
            filepath, filename, mime, size, url = result
            output = {
                "filepath": filepath,
                "filename": filename,
                "mimetype": mime,
                "size": size,
                "original_url": url,
            }
            print(json.dumps(output))
        else:
            print("Error: content not found", file=sys.stderr)
            sys.exit(1)

    except Exception as e:
        # Print errors to stderr so Rust can distinguish them from valid output
        print(f"Error: {str(e)}", file=sys.stderr)
        sys.exit(1)
