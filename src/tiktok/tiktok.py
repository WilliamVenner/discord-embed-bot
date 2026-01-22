import asyncio
import json
import re
import tempfile

from typing import Optional, Union

import aiofiles

from utilities.constants import DownloadQualities, Mimetyper, Mimetypes
from utilities.http import Proxy
from utilities.media.ffmpeg import FFMPEGManager
from utilities.tools.pyvips import resize_image_buffer

from utilities.tools.download.exceptions import (
    DownloadError,
    DownloadErrorAgeRestricted,
)
from utilities.tools.download.tools import maybe_get_cached_file, verify_file_quality

from .base import BaseService


class TikTokService(BaseService):
    _NAME = "tiktok"
    _VALID_URL = r"https?://(?:(?:vt|vm|m|t|www)\.)?tiktok\.com/(?:(?P<user>[^/]+)/(?:video|photo)/(?P<postId>[^/?#&]+)|i18n/share/video/(?P<sharePostId>[^/?#&]+)|(?P<shortLink>[^/?#&]+)|t/(?P<tShortLink>[^/?#&]+)|v/(?P<vPostId>[^./?#&]+)\.html)"

    _HEADERS = {
        "referer": "https://www.tiktok.com/",
        "user-agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    }
    _SHORT_DOMAIN = "https://vt.tiktok.com/"

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

        proxy = Proxy.get_proxy(special=True)

        post_id = (
            match.group("postId")
            or match.group("sharePostId")
            or match.group("vPostId")
        )

        short_link = match.group("shortLink") or match.group("tShortLink")
        if short_link and not post_id:
            post_id = await cls._resolve_short_link(short_link, proxy=proxy)

        if not post_id:
            raise DownloadError("Unable to extract post id from URL")

        details = await cls._get_video_detail(post_id, proxy=proxy)
        if not details:
            raise DownloadError("Unable to find any media in this url")
        detail, cookies = details

        audio_url = None
        image_url = None
        video_url = None

        should_resize = False

        if detail.get("video") and detail["video"]["playAddr"]:
            # match detail['video']['bitrateInfo'][]['PlayAddr']['dataSize'] to max_file_size
            # match detail['video']['bitrateInfo'][]['PlayAddr']['Height'] and ['Width'] to quality
            # use detail['video']['bitrateInfo'][]['PlayAddr']['Height'] and ['Width'] to quality
            # video_url = detail['video']['playAddr']
            video = cls.find_best_format(
                detail["video"]["bitrateInfo"],
                max_file_size=max_file_size,
                quality=quality,
            )
            video_url = video["PlayAddr"]["UrlList"][0]
            position = 0
        elif detail.get("imagePost"):
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

        # now that position has been cleaned, re-check cache
        response_cached = maybe_get_cached_file(
            media_url,
            audio_priority=audio_priority,
            max_file_size=max_file_size,
            quality=quality,
            position=position,
        )
        if response_cached is not None:
            return response_cached

        if audio_url:
            audio_mimetype = Mimetypes.AUDIO_AAC
            if "mime_type=audio_mpeg" in audio_url:
                audio_mimetype = Mimetypes.AUDIO_MP3

            audio_response, image_response = None, None
            if image_url:
                audio_response, image_response = await asyncio.gather(
                    Proxy.request_primp(
                        "get",
                        audio_url,
                        cookies=cookies,
                        headers=cls._HEADERS,
                        proxy=proxy,
                    ),
                    Proxy.request_primp(
                        "get",
                        image_url,
                        cookies=cookies,
                        headers=cls._HEADERS,
                        proxy=proxy,
                    ),
                )
                audio_buffer, image_buffer = await asyncio.gather(
                    audio_response.buffer(),
                    image_response.buffer(),
                )

                image_mimetype = Mimetyper.get_mimetype_from_filename(image_url)

                if quality and should_resize:
                    dimension = int(quality.value[0:-1])
                    image_buffer, image_mimetype, _ = resize_image_buffer(
                        image_mimetype,
                        image_buffer,
                        convert=Mimetypes.IMAGE_PNG,
                        resize_size=[dimension, -1],
                    )

                audio_manager, image_manager = await asyncio.gather(
                    FFMPEGManager.create(audio_buffer, audio_mimetype),
                    FFMPEGManager.create(image_buffer, image_mimetype, loop=1),
                )
                image_manager.stream_audio = audio_manager.stream

                buffer, mimetype, _ = await image_manager.to_buffer(
                    Mimetypes.VIDEO_MP4,
                    pix_fmt="yuv420p",
                    color_range="tv",
                    r=1,
                    t=(audio_manager.metadata.duration / 1000),
                    tune="stillimage",
                    **{"level:v": "3.2"},
                )
                extension = Mimetyper.get_extension_from_mimetype(mimetype)

                await audio_manager.close()

                """
                response = await Proxy.request_primp('get', image_url, cookies=cookies, headers=cls._HEADERS, use_special_proxy=True)
                buffer = await response.buffer()
                mimetype = Mimetyper.get_mimetype_from_filename(image_url)
                """
            else:
                response = await Proxy.request_primp(
                    "get", audio_url, cookies=cookies, headers=cls._HEADERS, proxy=proxy
                )
                buffer = await response.buffer()
                mimetype = audio_mimetype
        elif image_url:
            response = await Proxy.request_primp(
                "get", image_url, cookies=cookies, headers=cls._HEADERS, proxy=proxy
            )
            buffer = await response.buffer()
            mimetype = Mimetyper.get_mimetype_from_filename(image_url)
        elif video_url:
            response = await Proxy.request_primp(
                "get", video_url, cookies=cookies, headers=cls._HEADERS, proxy=proxy
            )
            buffer = await response.buffer()
            mimetype = Mimetypes.VIDEO_MP4
            # todo: maybe convert this to h264 if its h265?
        else:
            raise DownloadError("Unable to find any media in this post")

        extension = Mimetyper.get_extension_from_mimetype(mimetype)
        filename = f"services-{cls._NAME}-{post_id}-{position}.{extension}"
        filepath = tempfile.gettempdir() + "/" + filename
        async with aiofiles.open(filepath, mode="wb") as f:
            await f.write(buffer)

        return filepath, filename, mimetype.value, len(buffer), media_url

    @classmethod
    async def _resolve_short_link(
        cls, short_link: str, proxy: Optional[str] = None
    ) -> Optional[str]:
        try:
            url = cls._SHORT_DOMAIN + short_link
            headers = {
                "user-agent": cls._HEADERS["user-agent"].split(" Chrome/1")[0],
            }

            response = await Proxy.request_primp(
                "get", url, headers=headers, allow_redirects=False, proxy=proxy
            )
            html = await response.text()
            if html.startswith('<a href="https://'):
                extracted_url = html.split('<a href="')[1].split("?")[0]

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
    async def _get_video_detail(
        cls, post_id: str, proxy: Optional[str] = None
    ) -> Optional[tuple[dict, dict]]:
        """Get video detail from TikTok"""
        try:
            url = f"https://www.tiktok.com/@i/video/{post_id}"
            response = await Proxy.request_primp(
                "get", url, headers=cls._HEADERS, proxy=proxy
            )
            if not response.ok:
                return None

            html = await response.text()
            try:
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
                    raise DownloadErrorAgeRestricted()

                return detail, response.cookies
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

        for i in range(len(formats)):
            format = formats[i]
            if max_file_size and max_file_size < int(format["PlayAddr"]["DataSize"]):
                continue

            width, height = format["PlayAddr"]["Width"], format["PlayAddr"]["Height"]
            score = (
                width + height - format["QualityType"]
            )  # lower QualityType is better quality i think
            if quality:
                is_maybe_quality, is_quality = verify_file_quality(
                    width, height, quality
                )
                if not is_maybe_quality:
                    continue
                if is_quality:
                    score *= 10000

            if best_score < score:
                best_format = format
                best_score = score

        if not best_format and max_file_size:
            raise DownloadError(f"File is larger than {max_file_size} bytes")

        if not best_format:
            raise DownloadError(f"Unknown Error while retrieving file formats")

        return best_format
